//! Native JIT for trusted stage-0 scalar source. Never treat JIT as a sandbox.
use crate::frontend::{Expr, Program};
use crate::isolate::{IsolateHandle, OresVM};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::instructions::BlockArg;
use cranelift_codegen::ir::{AbiParam, Block, InstBuilder, MemFlagsData, Value, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

pub struct Compiled {
    module: JITModule, // Own executable allocations until after every native call.
    id: cranelift_module::FuncId,
    arity: usize,
    #[cfg(test)]
    pub clif: String,
}

// All signed remainder paths guard the dangerous (MIN, -1) pair before
// lowering to native srem. Both operands are evaluated exactly once first.
fn checked_rem_values(
    left: Value,
    right: Value,
    b: &mut FunctionBuilder,
    divisor_error: Block,
) -> Value {
    let is_zero = b.ins().icmp_imm_s(IntCC::Equal, right, 0);
    let nonzero = b.create_block();
    b.ins().brif(is_zero, divisor_error, &[], nonzero, &[]);
    b.switch_to_block(nonzero);
    b.seal_block(nonzero);

    let min = b.ins().icmp_imm_s(IntCC::Equal, left, i64::MIN);
    let neg_one = b.ins().icmp_imm_s(IntCC::Equal, right, -1);
    let special = b.ins().band(min, neg_one);
    let min_minus_one = b.create_block();
    let general = b.create_block();
    let done = b.create_block();
    b.append_block_param(done, types::I64);
    b.ins().brif(special, min_minus_one, &[], general, &[]);

    b.switch_to_block(min_minus_one);
    b.seal_block(min_minus_one);
    let zero = b.ins().iconst(types::I64, 0);
    b.ins().jump(done, &[BlockArg::Value(zero)]);

    b.switch_to_block(general);
    b.seal_block(general);
    let rem = b.ins().srem(left, right);
    b.ins().jump(done, &[BlockArg::Value(rem)]);

    b.switch_to_block(done);
    b.seal_block(done);
    b.block_params(done)[0]
}

// Iterative exponentiation by squaring, with checked multiplication at every
// required step. Squaring is skipped after the final exponent bit to prevent
// spurious overflow for e.g. Integer.pow(-2, 63) or pow(0, i64::MAX).
fn checked_pow_values(
    base: Value,
    exponent: Value,
    b: &mut FunctionBuilder,
    overflow_error: Block,
    negative_exponent_error: Block,
) -> Value {
    let neg = b.ins().icmp_imm_s(IntCC::SignedLessThan, exponent, 0);
    let admitted = b.create_block();
    b.ins()
        .brif(neg, negative_exponent_error, &[], admitted, &[]);
    b.switch_to_block(admitted);
    b.seal_block(admitted);

    let loop_head = b.create_block();
    for _ in 0..3 {
        b.append_block_param(loop_head, types::I64);
    }
    let loop_body = b.create_block();
    let odd = b.create_block();
    let even = b.create_block();
    let joined = b.create_block();
    b.append_block_param(joined, types::I64);
    let square = b.create_block();
    let finished = b.create_block();
    b.append_block_param(finished, types::I64);

    let one = b.ins().iconst(types::I64, 1);
    b.ins().jump(
        loop_head,
        &[
            BlockArg::Value(one),
            BlockArg::Value(base),
            BlockArg::Value(exponent),
        ],
    );
    b.switch_to_block(loop_head);
    let state = b.block_params(loop_head).to_vec();
    let done = b.ins().icmp_imm_s(IntCC::Equal, state[2], 0);
    b.ins()
        .brif(done, finished, &[BlockArg::Value(state[0])], loop_body, &[]);

    b.switch_to_block(loop_body);
    b.seal_block(loop_body);
    let bit = b.ins().band_imm_u(state[2], 1);
    let multiply = b.ins().icmp_imm_s(IntCC::NotEqual, bit, 0);
    b.ins().brif(multiply, odd, &[], even, &[]);

    b.switch_to_block(odd);
    b.seal_block(odd);
    let (product, overflow) = b.ins().smul_overflow(state[0], state[1]);
    let product_ok = b.create_block();
    b.ins().brif(overflow, overflow_error, &[], product_ok, &[]);
    b.switch_to_block(product_ok);
    b.seal_block(product_ok);
    b.ins().jump(joined, &[BlockArg::Value(product)]);

    b.switch_to_block(even);
    b.seal_block(even);
    b.ins().jump(joined, &[BlockArg::Value(state[0])]);

    b.switch_to_block(joined);
    b.seal_block(joined);
    let result = b.block_params(joined)[0];
    let remaining = b.ins().ushr_imm_u(state[2], 1);
    let final_bit = b.ins().icmp_imm_s(IntCC::Equal, remaining, 0);
    b.ins()
        .brif(final_bit, finished, &[BlockArg::Value(result)], square, &[]);

    b.switch_to_block(square);
    b.seal_block(square);
    let (base_sq, overflow) = b.ins().smul_overflow(state[1], state[1]);
    let square_ok = b.create_block();
    b.ins().brif(overflow, overflow_error, &[], square_ok, &[]);
    b.switch_to_block(square_ok);
    b.seal_block(square_ok);
    b.ins().jump(
        loop_head,
        &[
            BlockArg::Value(result),
            BlockArg::Value(base_sq),
            BlockArg::Value(remaining),
        ],
    );
    b.seal_block(loop_head);

    b.switch_to_block(finished);
    b.seal_block(finished);
    b.block_params(finished)[0]
}

fn validate_guest_program(source: &Program) -> Result<(), String> {
    if source.parameters.len() > 4 {
        return Err("parameter count exceeds four before native codegen".into());
    }
    if source.locals.len() > 64 {
        return Err("runtime local count exceeds 64 before native codegen".into());
    }
    let mut nodes = 0_usize;
    for (index, initializer) in source.locals.iter().enumerate() {
        validate_guest_expr(initializer, index, source.parameters.len(), &mut nodes)?;
    }
    validate_guest_expr(
        &source.result,
        source.locals.len(),
        source.parameters.len(),
        &mut nodes,
    )
}

// Revalidate every reference and budget even if a host constructs Program
// directly without the frontend. No forged index reaches Cranelift emission.
fn validate_guest_expr(
    root: &Expr,
    admitted_locals: usize,
    admitted_params: usize,
    nodes: &mut usize,
) -> Result<(), String> {
    let mut work = vec![(root, 1_usize)];
    while let Some((expr, depth)) = work.pop() {
        *nodes += 1;
        if *nodes > 4096 {
            return Err("AST node budget exceeded before native codegen".into());
        }
        if depth > 128 {
            return Err("expression depth budget exceeded before native codegen".into());
        }
        match expr {
            Expr::Literal(_) => {}
            Expr::Param(index) if *index < admitted_params => {}
            Expr::Local(index) if *index < admitted_locals => {}
            Expr::Seq(items) => {
                if items.is_empty() || items.len() > 16 {
                    return Err("E_SEQ_LIMIT: forged scalar sequence before native codegen".into());
                }
                for item in items.iter().rev() {
                    work.push((item, depth + 1));
                }
            }
            Expr::Param(_) => {
                return Err("invalid parameter reference before native codegen".into());
            }
            Expr::Local(_) => {
                return Err("invalid or forward local reference before native codegen".into());
            }
            Expr::Neg(x) | Expr::ShiftRight8(x) => work.push((x, depth + 1)),
            Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Mul(a, b)
            | Expr::BitXor(a, b)
            | Expr::BitAnd(a, b)
            | Expr::BitOr(a, b)
            | Expr::NamedRem(a, b)
            | Expr::NamedMod(a, b)
            | Expr::NamedPow(a, b) => {
                work.push((b, depth + 1));
                work.push((a, depth + 1));
            }
        }
    }
    Ok(())
}

fn emit(
    e: &Expr,
    b: &mut FunctionBuilder,
    params: &[cranelift_codegen::ir::Value],
    locals: &[cranelift_codegen::ir::Value],
    error: Block,
    divisor_error: Block,
    negative_exponent_error: Block,
) -> Value {
    use Expr::*;
    match e {
        Literal(v) => b.ins().iconst(types::I64, *v),
        Param(i) => params[*i],
        Local(i) => locals[*i],
        Seq(parts) => {
            // The source/host-AST gate guarantees a nonempty bounded list.
            // Emit every member: checked errors on discarded entries MUST
            // still branch before the result-store instruction.
            let (first, rest) = parts.split_first().expect("validated nonempty seq");
            let mut value = emit(
                first,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            for part in rest {
                value = emit(
                    part,
                    b,
                    params,
                    locals,
                    error,
                    divisor_error,
                    negative_exponent_error,
                );
            }
            value
        }
        ShiftRight8(x) => {
            // Signed Cranelift arithmetic shift of i64, preserving sign.
            // The parser admits only the constant 8, matching ODB1 byte
            // extraction for 0..i31::MAX without float narrowing.
            let input = emit(
                x,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            let eight = b.ins().iconst(types::I64, 8);
            b.ins().sshr(input, eight)
        }
        BitXor(x, y) | BitAnd(x, y) | BitOr(x, y) => {
            // Native signed-i64 bitwise primitives, not host-side emulation.
            // Emit and check each operand in lexical order before the bitwise
            // instruction; a checked arithmetic failure cannot be skipped.
            let lhs = emit(
                x,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            let rhs = emit(
                y,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            match e {
                BitXor(_, _) => b.ins().bxor(lhs, rhs),
                BitAnd(_, _) => b.ins().band(lhs, rhs),
                BitOr(_, _) => b.ins().bor(lhs, rhs),
                _ => unreachable!(),
            }
        }
        NamedRem(x, y) | NamedMod(x, y) => {
            let left = emit(
                x,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            let right = emit(
                y,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            let rem = checked_rem_values(left, right, b, divisor_error);
            if matches!(e, NamedRem(_, _)) {
                rem
            } else {
                // Nonnegative Euclidean modulus. The selected adjustment is
                // representable even for divisor == i64::MIN; no abs(MIN).
                let positive = b.ins().icmp_imm_s(IntCC::SignedGreaterThan, right, 0);
                let plus = b.ins().iadd(rem, right);
                let minus = b.ins().isub(rem, right);
                let adjusted = b.ins().select(positive, plus, minus);
                let negative = b.ins().icmp_imm_s(IntCC::SignedLessThan, rem, 0);
                b.ins().select(negative, adjusted, rem)
            }
        }
        NamedPow(x, y) => {
            let base = emit(
                x,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            let exponent = emit(
                y,
                b,
                params,
                locals,
                error,
                divisor_error,
                negative_exponent_error,
            );
            checked_pow_values(base, exponent, b, error, negative_exponent_error)
        }
        _ => {
            let (value, overflow) = match e {
                Neg(x) => {
                    let rhs = emit(
                        x,
                        b,
                        params,
                        locals,
                        error,
                        divisor_error,
                        negative_exponent_error,
                    );
                    let zero = b.ins().iconst(types::I64, 0);
                    b.ins().ssub_overflow(zero, rhs)
                }
                Add(x, y) => {
                    let lhs = emit(
                        x,
                        b,
                        params,
                        locals,
                        error,
                        divisor_error,
                        negative_exponent_error,
                    );
                    let rhs = emit(
                        y,
                        b,
                        params,
                        locals,
                        error,
                        divisor_error,
                        negative_exponent_error,
                    );
                    b.ins().sadd_overflow(lhs, rhs)
                }
                Sub(x, y) => {
                    let lhs = emit(
                        x,
                        b,
                        params,
                        locals,
                        error,
                        divisor_error,
                        negative_exponent_error,
                    );
                    let rhs = emit(
                        y,
                        b,
                        params,
                        locals,
                        error,
                        divisor_error,
                        negative_exponent_error,
                    );
                    b.ins().ssub_overflow(lhs, rhs)
                }
                Mul(x, y) => {
                    let lhs = emit(
                        x,
                        b,
                        params,
                        locals,
                        error,
                        divisor_error,
                        negative_exponent_error,
                    );
                    let rhs = emit(
                        y,
                        b,
                        params,
                        locals,
                        error,
                        divisor_error,
                        negative_exponent_error,
                    );
                    b.ins().smul_overflow(lhs, rhs)
                }
                _ => unreachable!(),
            };
            let next = b.create_block();
            b.ins().brif(overflow, error, &[], next, &[]);
            b.switch_to_block(next);
            b.seal_block(next);
            value
        }
    }
}

fn validate_ir(
    func: &cranelift_codegen::ir::Function,
    isa: &dyn cranelift_codegen::isa::TargetIsa,
) -> Result<(), String> {
    cranelift_codegen::verify_function(func, isa)
        .map_err(|e| format!("CLIF verification failed: {e}"))
}

fn prepare_ir(
    source: &Program,
) -> Result<
    (
        JITModule,
        cranelift_codegen::Context,
        cranelift_module::FuncId,
    ),
    String,
> {
    validate_guest_program(source)?;
    let builder = JITBuilder::new(default_libcall_names()).map_err(|e| e.to_string())?;
    let mut module = JITModule::new(builder);
    let mut sig = module.make_signature();
    // Stable internal entry ABI: four i64 values + output pointer -> i32 status.
    // 0 succeeds; 1 is checked overflow, 2 divisor zero, 3 negative exponent.
    // All failures leave output unchanged.
    for _ in 0..4 {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.params
        .push(AbiParam::new(module.target_config().pointer_type()));
    sig.returns.push(AbiParam::new(types::I32));
    let id = module
        .declare_function("oreslang_scalar_entry", Linkage::Local, &sig)
        .map_err(|e| e.to_string())?;
    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fbctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
    let entry = b.create_block();
    let failure = b.create_block();
    let divisor_error = b.create_block();
    let negative_exponent_error = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let p = b.block_params(entry).to_vec();
    let mut locals = Vec::with_capacity(source.locals.len());
    for initializer in &source.locals {
        // A runtime val initializer runs once in lexical order, even if it
        // is unused; checked traps branch away before any output store.
        let value = emit(
            initializer,
            &mut b,
            &p[..4],
            &locals,
            failure,
            divisor_error,
            negative_exponent_error,
        );
        locals.push(value);
    }
    let value = emit(
        &source.result,
        &mut b,
        &p[..4],
        &locals,
        failure,
        divisor_error,
        negative_exponent_error,
    );
    b.ins().store(MemFlagsData::new(), value, p[4], 0);
    let ok = b.ins().iconst(types::I32, 0);
    b.ins().return_(&[ok]);
    b.switch_to_block(failure);
    let fail = b.ins().iconst(types::I32, 1);
    b.ins().return_(&[fail]);
    b.switch_to_block(divisor_error);
    let divisor_zero = b.ins().iconst(types::I32, 2);
    b.ins().return_(&[divisor_zero]);
    b.switch_to_block(negative_exponent_error);
    let negative = b.ins().iconst(types::I32, 3);
    b.ins().return_(&[negative]);
    b.seal_all_blocks();
    b.finalize(module.target_config());
    // Unconditional SSA, CFG, type and ABI verification before native emission.
    // This must not depend on Cranelift's optional enable_verifier setting.
    validate_ir(&ctx.func, module.isa())?;
    Ok((module, ctx, id))
}

// Constructs and verifies the same CLIF as the native compiler without
// define_function, finalize_definitions, or executable-code allocation.
pub fn render_ir(source: &Program) -> Result<String, String> {
    let (_module, ctx, _id) = prepare_ir(source)?;
    Ok(ctx.func.display().to_string())
}

pub fn compile(source: &Program) -> Result<Compiled, String> {
    let (mut module, mut ctx, id) = prepare_ir(source)?;
    #[cfg(test)]
    let clif = ctx.func.display().to_string();
    module
        .define_function(id, &mut ctx)
        .map_err(|e| e.to_string())?;
    module.clear_context(&mut ctx);
    module.finalize_definitions().map_err(|e| e.to_string())?;
    Ok(Compiled {
        module,
        id,
        arity: source.parameters.len(),
        #[cfg(test)]
        clif,
    })
}

impl Compiled {
    // This is a trusted, host-mediated tenant step, not a native pointer ABI.
    // The same compiled machine code is reused for multiple IsolateVMs while
    // mutable scalar state remains owned by the OresVM tenant host.
    pub fn run_for_isolate(
        &self,
        vm: &OresVM,
        handle: &IsolateHandle,
        delta: i64,
    ) -> Result<i64, String> {
        if self.arity != 2 {
            return Err("isolate scalar entry requires (old, delta)".into());
        }
        vm.invoke_trusted_step(handle, delta, |old, change| self.run(&[old, change]))
    }

    pub fn run(&self, args: &[i64]) -> Result<i64, String> {
        if args.len() > 4 {
            return Err("too many runtime arguments".into());
        }
        // Enforce source-entry arity at this API boundary too. The CLI checks
        // count, but direct host / tenant calls must not silently bind absent
        // parameters to ABI padding zeros or ignore surplus arguments.
        if args.len() != self.arity {
            return Err("runtime argument count mismatch".into());
        }
        let mut padded = [0_i64; 4];
        padded[..args.len()].copy_from_slice(args);
        let code = self.module.get_finalized_function(self.id);
        // SAFETY: emitted signature is exactly the C ABI below, the Module
        // owns the executable code for this entire call, and output is valid.
        // The caller MUST treat source as trusted; this is NOT an isolation
        // boundary for hostile Oreslang code or actor heaps.
        type Entry = unsafe extern "C" fn(i64, i64, i64, i64, *mut i64) -> i32;
        let function: Entry = unsafe { std::mem::transmute(code) };
        let mut out = 0_i64;
        let status = unsafe { function(padded[0], padded[1], padded[2], padded[3], &mut out) };
        if status == 0 {
            Ok(out)
        } else if status == 1 {
            Err("i64 arithmetic overflow".into())
        } else if status == 2 {
            Err("integer divisor zero".into())
        } else if status == 3 {
            Err("negative integer exponent".into())
        } else {
            Err("invalid generated native status".into())
        }
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;

    #[test]
    fn ir_render_does_not_require_native_finalization_and_matches_compiled_clif() {
        let parsed = crate::frontend::parse("fnc main(a:int):int{return Integer.bitXor(a+40,2);}")
            .expect("valid typed scalar");
        let emitted_only = render_ir(&parsed).expect("verified non-executable CLIF");
        assert!(emitted_only.contains("function"));
        let native = compile(&parsed).expect("Cranelift executable output");
        assert_eq!(emitted_only, native.clif);
        assert_eq!(native.run(&[0]), Ok(42));
        let bad = crate::frontend::parse("fnc main():int{return 5 ^ 3;}");
        assert!(bad.is_err(), "unsupported operator must fail before IR");
    }

    #[test]
    fn direct_compiled_api_requires_exact_declared_arity() {
        let program = crate::frontend::parse("fnc main(a:int,b:int):int{return a*10+b;}")
            .expect("two-parameter native scalar");
        let compiled = compile(&program).expect("compile exact arity fixture");
        assert_eq!(compiled.run(&[4, 2]), Ok(42));
        for args in [&[][..], &[4][..], &[4, 2, 0][..], &[4, 2, 0, 0][..]] {
            assert_eq!(
                compiled.run(args),
                Err("runtime argument count mismatch".into())
            );
        }
        assert_eq!(
            compiled.run(&[4, 2, 0, 0, 0]),
            Err("too many runtime arguments".into())
        );
        assert_eq!(
            compiled.run(&[4, 2]),
            Ok(42),
            "failed admission cannot corrupt native state"
        );
    }

    #[test]
    fn malformed_clif_is_rejected_before_native_codegen() {
        let builder = JITBuilder::new(default_libcall_names()).unwrap();
        let module = JITModule::new(builder);
        let mut func = cranelift_codegen::ir::Function::new();
        // A signature requiring an i64 return cannot legally return zero values.
        func.signature.returns.push(AbiParam::new(types::I64));
        let mut fbctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut func, &mut fbctx);
        let entry = b.create_block();
        b.switch_to_block(entry);
        b.seal_block(entry);
        b.ins().return_(&[]);
        b.finalize(module.target_config());

        let error = validate_ir(&func, module.isa())
            .expect_err("malformed CLIF must fail independently of codegen settings");
        assert!(error.starts_with("CLIF verification failed:"), "{error}");
    }
}

#[cfg(test)]
mod native_named_signed64_differential {
    use super::*;
    use crate::frontend::parse;

    #[test]
    fn native_composed_named_i64_matches_java_net_and_llvm_reference() {
        // These expressions execute native code generated from guest source.
        // The same guest compositions are expected by Java and .NET; the
        // LLVM reference and C-generated WASM ABI tests check equivalent math.
        for (expr, expected) in [
            ("Integer.bitXor(Integer.mod(-9,4),Integer.pow(2,3))", 11_i64),
            ("Integer.rem(Integer.bitXor(5,3),Integer.pow(2,2))", 2),
            ("Integer.pow(Integer.rem(-9,4)+3,3)", 8),
            ("Integer.mod(Integer.pow(-2,3),5)", 2),
            ("Integer.pow(-2,63)", i64::MIN),
            ("Integer.pow(-1,9223372036854775807)", -1),
        ] {
            let source = format!("fnc main():int{{return {expr};}}");
            let parsed = parse(&source).expect("composed typed i64 guest");
            let compiled = compile(&parsed).expect("native Cranelift machine code");
            assert_eq!(compiled.run(&[]), Ok(expected), "guest {source}");
        }
    }

    #[test]
    fn failed_native_arithmetic_never_publishes_partial_output() {
        // Exercise the real generated entry ABI directly. Rejected guest
        // calls must return an error status and leave the caller's output
        // sentinel unchanged; no host-only evaluator can prove this.
        type Entry = unsafe extern "C" fn(i64, i64, i64, i64, *mut i64) -> i32;
        for (source, args, expected_status) in [
            (
                "fnc main(a:int,b:int):int{return Integer.rem(a,b);}",
                [9_i64, 0, 0, 0],
                2_i32,
            ),
            (
                "fnc main(a:int,b:int):int{return Integer.mod(a+1,b);}",
                [i64::MAX, 0, 0, 0],
                1_i32,
            ),
            (
                "fnc main(a:int,b:int):int{return Integer.pow(a,b);}",
                [2, 63, 0, 0],
                1_i32,
            ),
            (
                "fnc main(a:int,b:int):int{return Integer.pow(a,b);}",
                [2, -1, 0, 0],
                3_i32,
            ),
        ] {
            let parsed = parse(source).expect("typed scalar expression");
            let native = compile(&parsed).expect("actual native code");
            let function: Entry =
                unsafe { std::mem::transmute(native.module.get_finalized_function(native.id)) };
            let mut output = 0x1122_3344_5566_7788_i64;
            let status = unsafe { function(args[0], args[1], args[2], args[3], &mut output) };
            assert_eq!(status, expected_status, "error status for {source}");
            assert_eq!(
                output, 0x1122_3344_5566_7788_i64,
                "failed native arithmetic published a partial value: {source}"
            );
        }
    }

    #[test]
    fn native_dynamic_rem_mod_match_independent_i128_oracle() {
        // Actual compiled Cranelift entry points, with dynamic signed64
        // numerator AND denominator. The expected results are calculated
        // with wider signed i128, not the frontend or a production helper.
        let divisors = [i64::MIN, i64::MIN + 1, -17, -4, -1, 0, 1, 3, 17, i64::MAX];
        let numerators = [
            i64::MIN,
            i64::MIN + 1,
            -i64::MAX,
            -99,
            -9,
            -1,
            0,
            1,
            9,
            99,
            i64::MAX - 1,
            i64::MAX,
        ];
        for op in ["rem", "mod"] {
            let source = format!("fnc main(a:int,b:int):int{{return Integer.{op}(a,b);}}");
            let program = parse(&source).expect("dynamic named arithmetic parses");
            let compiled = compile(&program).expect("native named arithmetic compiles");
            for divisor in divisors {
                let mut seed = 0x9e37_79b9_7f4a_7c15_u64 ^ (divisor as u64);
                let random = (0..128).map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as i64
                });
                for numerator in numerators.into_iter().chain(random) {
                    let actual = compiled.run(&[numerator, divisor]);
                    if divisor == 0 {
                        assert_eq!(
                            actual.as_ref().map(|_| ()).map_err(String::as_str),
                            Err("integer divisor zero"),
                            "{op}({numerator},{divisor})"
                        );
                        continue;
                    }
                    let rem = (numerator as i128) % (divisor as i128);
                    let expected = if op == "mod" && rem < 0 {
                        rem + (divisor as i128).abs()
                    } else {
                        rem
                    };
                    let expected = i64::try_from(expected).expect("signed64 result");
                    assert_eq!(
                        actual,
                        Ok(expected),
                        "Cranelift native {op}({numerator},{divisor})"
                    );
                }
            }
        }
    }

    #[test]
    fn native_pow_matches_checked_i128_oracle_and_error_precedence() {
        // A finite bounded matrix includes nonnegative exponents, negative
        // exponents, exact MIN, and early checked overflow. This executes
        // generated pow control flow, not the frontend-only evaluator.
        let program =
            parse("fnc main(base:int,exponent:int):int{return Integer.pow(base,exponent);}")
                .expect("named pow source");
        let compiled = compile(&program).expect("native pow JIT");
        for base in [i64::MIN, -17, -3, -2, -1, 0, 1, 2, 3, 17, i64::MAX] {
            for exponent in [-3_i64, -1, 0, 1, 2, 3, 5, 16, 31, 63, 64, 127] {
                let actual = compiled.run(&[base, exponent]);
                if exponent < 0 {
                    assert_eq!(
                        actual.as_ref().map(|_| ()).map_err(String::as_str),
                        Err("negative integer exponent"),
                        "native pow({base},{exponent})"
                    );
                    continue;
                }
                let mut expected = 1_i128;
                let mut failure = false;
                for _ in 0..exponent {
                    // Stop when the signed64 result is no longer representable:
                    // continue squaring would not add any relevant evidence.
                    if let Some(result) = expected.checked_mul(base as i128) {
                        if result < i64::MIN as i128 || result > i64::MAX as i128 {
                            failure = true;
                            break;
                        }
                        expected = result;
                    } else {
                        failure = true;
                        break;
                    }
                }
                if failure {
                    assert_eq!(
                        actual.as_ref().map(|_| ()).map_err(String::as_str),
                        Err("i64 arithmetic overflow"),
                        "native pow({base},{exponent})"
                    );
                } else {
                    assert_eq!(actual, Ok(expected as i64), "native pow({base},{exponent})");
                }
            }
        }
        for (base, exponent, expected) in [
            (0, i64::MAX, 0),
            (1, i64::MAX, 1),
            (-1, i64::MAX, -1),
            (-1, i64::MAX - 1, 1),
            (-2, 63, i64::MIN),
        ] {
            assert_eq!(
                compiled.run(&[base, exponent]),
                Ok(expected),
                "native large exponent pow({base},{exponent})"
            );
        }
    }
}

#[cfg(test)]
mod local_ast_security_tests {
    use super::*;

    #[test]
    fn host_constructed_forward_or_forged_references_fail_before_clif() {
        let self_reference = Program {
            parameters: vec![],
            locals: vec![Expr::Local(0)],
            result: Expr::Literal(42),
        };
        assert!(
            render_ir(&self_reference)
                .unwrap_err()
                .contains("invalid or forward local reference")
        );

        let forged_parameter = Program {
            parameters: vec![],
            locals: vec![],
            result: Expr::Param(0),
        };
        assert!(
            render_ir(&forged_parameter)
                .unwrap_err()
                .contains("invalid parameter reference")
        );
    }

    #[test]
    fn direct_ast_limits_reject_before_jit() {
        let too_many = Program {
            parameters: vec![],
            locals: vec![Expr::Literal(1); 65],
            result: Expr::Literal(42),
        };
        assert!(
            render_ir(&too_many)
                .unwrap_err()
                .contains("runtime local count exceeds 64")
        );

        let mut deep = Expr::Literal(42);
        for _ in 0..129 {
            deep = Expr::Neg(Box::new(deep));
        }
        let deep_program = Program {
            parameters: vec![],
            locals: vec![],
            result: deep,
        };
        assert!(
            render_ir(&deep_program)
                .unwrap_err()
                .contains("expression depth budget exceeded")
        );
    }
}

#[cfg(test)]
mod integrated_local_sequence_native_tests {
    use super::*;

    #[test]
    fn forged_sequence_and_local_references_are_denied_before_jit() {
        let malformed = [
            Expr::Seq(Vec::new()),
            Expr::Seq(vec![Expr::Literal(1); 17]),
            Expr::Seq(vec![Expr::Local(0)]),
            Expr::Seq(vec![Expr::Seq(vec![Expr::Local(99)])]),
        ];
        for expr in malformed {
            let program = Program {
                parameters: Vec::new(),
                locals: Vec::new(),
                result: expr,
            };
            assert!(render_ir(&program).is_err(), "forged AST emitted CLIF");
            assert!(
                compile(&program).is_err(),
                "forged AST compiled to native code"
            );
        }
    }

    #[test]
    fn combined_runtime_local_and_constexpr_seq_emit_checked_native_code() {
        let program = crate::frontend::parse(
            "fnc main(a:int):int{val live=seq(1,a+1);const answer=seq(1,40+2);return seq(live,answer);}"
        ).expect("integrated source must parse");
        let ir = render_ir(&program).expect("validate CLIF");
        let native = compile(&program).expect("compile real native guest");
        assert_eq!(
            native.clif, ir,
            "IR preview and native guest must share checked IR"
        );
        assert_eq!(native.run(&[40]), Ok(42));
        assert_eq!(
            native.run(&[i64::MAX]),
            Err("i64 arithmetic overflow".into())
        );
    }

    #[test]
    fn unused_runtime_sequence_must_still_trap() {
        let program =
            crate::frontend::parse("fnc main(a:int):int{val ignored=seq(a+1,42);return 42;}")
                .expect("checked runtime initializer should parse");
        let native = compile(&program).expect("native checked guest");
        assert_eq!(native.run(&[1]), Ok(42));
        assert_eq!(
            native.run(&[i64::MAX]),
            Err("i64 arithmetic overflow".into())
        );
    }
}
