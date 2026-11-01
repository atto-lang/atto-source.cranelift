//! Native JIT for trusted stage-0 scalar source. Never treat JIT as a sandbox.
use crate::frontend::{Expr, Program};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

pub struct Compiled {
    module: JITModule, // Own executable allocations until after every native call.
    id: cranelift_module::FuncId,
    pub clif: String,
}

fn emit(
    e: &Expr,
    b: &mut FunctionBuilder,
    params: &[cranelift_codegen::ir::Value],
    error: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    use Expr::*;
    match e {
        Literal(v) => b.ins().iconst(types::I64, *v),
        Param(i) => params[*i],
        BitXor(x, y) => {
            // Both operands are emitted in lexical order. bxor is an actual
            // native Cranelift instruction, not a host-side reference oracle.
            let lhs = emit(x, b, params, error);
            let rhs = emit(y, b, params, error);
            b.ins().bxor(lhs, rhs)
        }
        Rem(lhs, divisor) | Mod(lhs, divisor) => {
            // Evaluate the numerator first: a checked overflow is observable
            // even when rem/mod by -1 would otherwise yield the constant 0.
            let numerator = emit(lhs, b, params, error);
            assert_ne!(*divisor, 0, "internal invalid remainder divisor");
            // Native signed i64 MIN % -1 traps. Java's strict contract defines
            // that remainder to be zero, which is also Euclidean modulus zero.
            if *divisor == -1 {
                b.ins().iconst(types::I64, 0)
            } else {
                let denominator = b.ins().iconst(types::I64, *divisor);
                let remainder = b.ins().srem(numerator, denominator);
                if matches!(e, Mod(..)) {
                    let negative =
                        b.ins().icmp_imm_s(IntCC::SignedLessThan, remainder, 0);
                    // The candidate is only observed when remainder < 0.
                    // In that branch it lies in [0, |divisor|), including when
                    // divisor == i64::MIN. Cranelift integer arithmetic has
                    // defined modulo-2^64 intermediate semantics.
                    let adjusted = if *divisor > 0 {
                        b.ins().iadd(remainder, denominator)
                    } else {
                        b.ins().isub(remainder, denominator)
                    };
                    b.ins().select(negative, adjusted, remainder)
                } else {
                    remainder
                }
            }
        }
        _ => {
            let (value, overflow) = match e {
                Neg(x) => {
                    let rhs = emit(x, b, params, error);
                    let zero = b.ins().iconst(types::I64, 0);
                    b.ins().ssub_overflow(zero, rhs)
                }
                Add(x, y) => {
                    let lhs = emit(x, b, params, error);
                    let rhs = emit(y, b, params, error);
                    b.ins().sadd_overflow(lhs, rhs)
                }
                Sub(x, y) => {
                    let lhs = emit(x, b, params, error);
                    let rhs = emit(y, b, params, error);
                    b.ins().ssub_overflow(lhs, rhs)
                }
                Mul(x, y) => {
                    let lhs = emit(x, b, params, error);
                    let rhs = emit(y, b, params, error);
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

pub fn compile(source: &Program) -> Result<Compiled, String> {
    let builder = JITBuilder::new(default_libcall_names()).map_err(|e| e.to_string())?;
    let mut module = JITModule::new(builder);
    let mut sig = module.make_signature();
    // Stable internal entry ABI: four i64 values + output pointer -> i32 status.
    // 0 is success; 1 is *checked arithmetic overflow*, output unchanged.
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
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let p = b.block_params(entry).to_vec();
    let value = emit(&source.result, &mut b, &p[..4], failure);
    b.ins().store(MemFlagsData::new(), value, p[4], 0);
    let ok = b.ins().iconst(types::I32, 0);
    b.ins().return_(&[ok]);
    b.switch_to_block(failure);
    let fail = b.ins().iconst(types::I32, 1);
    b.ins().return_(&[fail]);
    b.seal_all_blocks();
    b.finalize(module.target_config());
    let clif = ctx.func.display().to_string();
    module
        .define_function(id, &mut ctx)
        .map_err(|e| e.to_string())?;
    module.clear_context(&mut ctx);
    module.finalize_definitions().map_err(|e| e.to_string())?;
    Ok(Compiled { module, id, clif })
}

impl Compiled {
    pub fn run(&self, args: &[i64]) -> Result<i64, String> {
        if args.len() > 4 {
            return Err("too many runtime arguments".into());
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
        } else {
            Err("invalid generated native status".into())
        }
    }
}

#[cfg(test)]
mod native_rem_mod_differential_tests {
    use super::*;
    use crate::frontend::parse;

    #[test]
    fn signed64_native_jit_matches_independent_i128_oracle() {
        // Compile one real executable per intrinsic and literal divisor,
        // then invoke it repeatedly with signed64 runtime arguments.
        // This is NOT merely a frontend/evaluator unit test.
        let divisors = [
            i64::MIN,
            -9223372036854775807,
            -17,
            -4,
            -1,
            1,
            3,
            17,
            i64::MAX,
        ];
        let numerators = [
            i64::MIN,
            i64::MIN + 1,
            -9223372036854775807,
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
            for divisor in divisors {
                let source = format!(
                    "fnc main(x:int):int{{return Integer.{op}(x,{divisor});}}"
                );
                let parsed = parse(&source).expect("literal denominator is supported");
                let native = compile(&parsed).expect("Cranelift JIT must lower subset");
                // Deterministic xorshift stream extends the handpicked
                // boundary vectors without nondeterministic CI seeds.
                // All generated values are tested against an independent
                // wider signed-i128 oracle, never the frontend evaluator.
                let mut seed = 0x9e37_79b9_7f4a_7c15_u64 ^ (divisor as u64);
                let random = (0..128).map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as i64
                });
                for numerator in numerators.into_iter().chain(random) {
                    let exact_remainder = (numerator as i128) % (divisor as i128);
                    let exact = if op == "mod" && exact_remainder < 0 {
                        exact_remainder + (divisor as i128).abs()
                    } else {
                        exact_remainder
                    };
                    let expected = i64::try_from(exact).expect("result is signed64");
                    assert_eq!(
                        native.run(&[numerator]).expect("guest must run"),
                        expected,
                        "native {op}({numerator}, {divisor})"
                    );
                }
            }
        }
    }
}
