//! Native JIT for trusted stage-0 scalar source. Never treat JIT as a sandbox.
use crate::frontend::{Expr, Program, TraceStep};
use crate::isolate::{IsolateHandle, OresVM};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::instructions::BlockArg;
use cranelift_codegen::ir::{
    AbiParam, Block, GlobalValue, InstBuilder, MemFlagsData, Value, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

mod telemetry;

pub struct Compiled {
    pub(crate) module: JITModule, // Own executable allocations until after every native call.
    pub(crate) id: cranelift_module::FuncId,
    pub(crate) arity: usize,
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
    let mut work = vec![(root, 1_usize, admitted_locals, admitted_params)];
    while let Some((expr, depth, admitted_locals, admitted_params)) = work.pop() {
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
                    work.push((item, depth + 1, admitted_locals, admitted_params));
                }
            }
            Expr::Param(_) => {
                return Err("invalid parameter reference before native codegen".into());
            }
            Expr::Local(_) => {
                return Err("invalid or forward local reference before native codegen".into());
            }
            Expr::ScalarLoop(args, writable, condition, steps, tail, body) => {
                if args.len() > 8 || args.len() + tail.len() > 64 {
                    return Err("E_UNIT_LIMIT: invalid loop frame".into());
                }
                let mut allowed = std::collections::BTreeSet::new();
                for i in writable {
                    if *i >= args.len() || !allowed.insert(*i) {
                        return Err("invalid loop writable slot".into());
                    }
                }
                for x in args {
                    work.push((x, depth + 1, admitted_locals, admitted_params));
                }
                work.push((condition, depth + 1, args.len(), 0));
                work.push((body, depth + 1, args.len() + tail.len(), 0));
                for (i, x) in tail.iter().enumerate() {
                    work.push((x, depth + 1, args.len() + i, 0));
                }
                let mut blocks = vec![(steps.as_slice(), args.len(), allowed, depth + 1)];
                while let Some((items, mut count, mut mutable, d)) = blocks.pop() {
                    if d > 128 || items.len() > 64 {
                        return Err("E_UNIT_LIMIT: loop statements".into());
                    }
                    for step in items {
                        *nodes += 1;
                        if *nodes > 4096 {
                            return Err("AST node budget exceeded".into());
                        }
                        use crate::frontend::LoopStep;
                        match step {
                            LoopStep::Bind(x, m) => {
                                if count >= 64 {
                                    return Err("loop local limit".into());
                                }
                                work.push((x, d + 1, count, 0));
                                if *m {
                                    mutable.insert(count);
                                }
                                count += 1;
                            }
                            LoopStep::Set(i, x) => {
                                if *i >= count || !mutable.contains(i) {
                                    return Err("invalid loop assignment".into());
                                }
                                work.push((x, d + 1, count, 0));
                            }
                            LoopStep::Exit(c, _) => work.push((c, d + 1, count, 0)),
                            LoopStep::Loop(c, ss) => {
                                work.push((c, d + 1, count, 0));
                                blocks.push((ss, count, mutable.clone(), d + 1));
                            }
                        }
                    }
                }
            }
            Expr::ScopedCall(args, callee_locals, body) => {
                if args.len() > 8 || args.len() + callee_locals.len() > 64 {
                    return Err("E_UNIT_LIMIT: invalid scoped helper frame".into());
                }
                // Arguments may see the caller; the isolated callee may see only
                // its own parameters and already initialized lexical slots.
                work.push((body, depth + 1, args.len() + callee_locals.len(), 0));
                for (index, init) in callee_locals.iter().enumerate().rev() {
                    work.push((init, depth + 1, args.len() + index, 0));
                }
                for arg in args.iter().rev() {
                    work.push((arg, depth + 1, admitted_locals, admitted_params));
                }
            }
            Expr::Sequence(prefix, last) => {
                if prefix.len() > 4096 {
                    return Err("AST node budget exceeded before native codegen".into());
                }
                work.push((last, depth + 1, admitted_locals, admitted_params));
                for part in prefix.iter().rev() {
                    work.push((part, depth + 1, admitted_locals, admitted_params));
                }
            }
            Expr::I32Checked(x) => work.push((x, depth + 1, admitted_locals, admitted_params)),
            Expr::RefinementCast(x, lo, hi) => {
                if lo > hi {
                    return Err("E_REFINEMENT_INTERVAL: forged interval".into());
                }
                work.push((x, depth + 1, admitted_locals, admitted_params));
            }
            Expr::Neg(x) | Expr::ShiftRight8(x) | Expr::InlineTask(x) => {
                work.push((x, depth + 1, admitted_locals, admitted_params))
            }
            Expr::Choose(c, y, n) => {
                work.push((c, depth + 1, admitted_locals, admitted_params));
                work.push((y, depth + 1, admitted_locals, admitted_params));
                work.push((n, depth + 1, admitted_locals, admitted_params));
            }
            Expr::Compare(op, a, b) => {
                if *op > 5 {
                    return Err("invalid comparison opcode before native codegen".into());
                }
                work.push((a, depth + 1, admitted_locals, admitted_params));
                work.push((b, depth + 1, admitted_locals, admitted_params));
            }
            Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Mul(a, b)
            | Expr::Div(a, b)
            | Expr::BitXor(a, b)
            | Expr::BitAnd(a, b)
            | Expr::BitOr(a, b)
            | Expr::NamedRem(a, b)
            | Expr::NamedMod(a, b)
            | Expr::NamedPow(a, b) => {
                work.push((b, depth + 1, admitted_locals, admitted_params));
                work.push((a, depth + 1, admitted_locals, admitted_params));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ScalarErrorBlocks {
    error: Block,
    divisor_error: Block,
    negative_exponent_error: Block,
    refinement_error: Block,
    i32_error: Block,
    unsigned: bool,
}

fn emit_scalar_loop(
    condition: &Expr,
    steps: &[crate::frontend::LoopStep],
    b: &mut FunctionBuilder<'_>,
    frame: &mut Vec<Value>,
    errors: ScalarErrorBlocks,
) {
    use crate::frontend::LoopStep;
    let count = frame.len();
    let header = b.create_block();
    let body = b.create_block();
    let exit = b.create_block();
    for _ in 0..count {
        b.append_block_param(header, types::I64);
        b.append_block_param(exit, types::I64);
    }
    let entry = frame
        .iter()
        .copied()
        .map(BlockArg::Value)
        .collect::<Vec<_>>();
    b.ins().jump(header, &entry);
    b.switch_to_block(header);
    let header_values = b.block_params(header).to_vec();
    let test = emit(condition, b, &[], &header_values, errors);
    let truth = b.ins().icmp_imm_s(IntCC::NotEqual, test, 0);
    let false_args = header_values
        .iter()
        .copied()
        .map(BlockArg::Value)
        .collect::<Vec<_>>();
    b.ins().brif(truth, body, &[], exit, &false_args);
    b.switch_to_block(body);
    b.seal_block(body);
    let mut current = header_values.clone();
    for step in steps {
        match step {
            LoopStep::Bind(x, _) => {
                let value = emit(x, b, &[], &current, errors);
                current.push(value);
            }
            LoopStep::Set(i, x) => {
                let value = emit(x, b, &[], &current, errors);
                current[*i] = value;
            }
            LoopStep::Loop(c, ss) => emit_scalar_loop(c, ss, b, &mut current, errors),
            LoopStep::Exit(c, breaking) => {
                let test = emit(c, b, &[], &current, errors);
                let truth = b.ins().icmp_imm_s(IntCC::NotEqual, test, 0);
                let next = b.create_block();
                let values = current[..count]
                    .iter()
                    .copied()
                    .map(BlockArg::Value)
                    .collect::<Vec<_>>();
                b.ins().brif(
                    truth,
                    if *breaking { exit } else { header },
                    &values,
                    next,
                    &[],
                );
                b.switch_to_block(next);
                b.seal_block(next);
            }
        }
    }
    let back = current[..count]
        .iter()
        .copied()
        .map(BlockArg::Value)
        .collect::<Vec<_>>();
    b.ins().jump(header, &back);
    b.seal_block(header);
    b.switch_to_block(exit);
    b.seal_block(exit);
    *frame = b.block_params(exit).to_vec();
}

fn emit(
    e: &Expr,
    b: &mut FunctionBuilder,
    params: &[cranelift_codegen::ir::Value],
    locals: &[cranelift_codegen::ir::Value],
    errors: ScalarErrorBlocks,
) -> Value {
    let ScalarErrorBlocks {
        error,
        divisor_error,
        negative_exponent_error,
        refinement_error,
        i32_error,
        unsigned,
    } = errors;
    use Expr::*;
    match e {
        Literal(v) => b.ins().iconst(types::I64, *v),
        Param(i) => params[*i],
        Local(i) => locals[*i],
        Compare(op, x, y) => {
            let a = emit(x, b, params, locals, errors);
            let z = emit(y, b, params, locals, errors);
            let cc = match op {
                0 => IntCC::Equal,
                1 => IntCC::NotEqual,
                2 => IntCC::SignedLessThan,
                3 => IntCC::SignedLessThanOrEqual,
                4 => IntCC::SignedGreaterThan,
                5 => IntCC::SignedGreaterThanOrEqual,
                _ => unreachable!(),
            };
            let cmp = b.ins().icmp(cc, a, z);
            b.ins().uextend(types::I64, cmp)
        }
        Choose(c, y, n) => {
            let test = emit(c, b, params, locals, errors);
            let yes = b.create_block();
            let no = b.create_block();
            let done = b.create_block();
            b.append_block_param(done, types::I64);
            let truth = b.ins().icmp_imm_s(IntCC::NotEqual, test, 0);
            b.ins().brif(truth, yes, &[], no, &[]);
            b.switch_to_block(yes);
            b.seal_block(yes);
            let a = emit(y, b, params, locals, errors);
            b.ins().jump(done, &[BlockArg::Value(a)]);
            b.switch_to_block(no);
            b.seal_block(no);
            let z = emit(n, b, params, locals, errors);
            b.ins().jump(done, &[BlockArg::Value(z)]);
            b.switch_to_block(done);
            b.seal_block(done);
            b.block_params(done)[0]
        }
        ScalarLoop(args, _, condition, steps, tail, body) => {
            let mut frame = args
                .iter()
                .map(|x| emit(x, b, params, locals, errors))
                .collect::<Vec<_>>();
            emit_scalar_loop(condition, steps, b, &mut frame, errors);
            for x in tail {
                let value = emit(x, b, &[], &frame, errors);
                frame.push(value);
            }
            emit(body, b, &[], &frame, errors)
        }
        ScopedCall(args, initializers, body) => {
            // The caller evaluates arguments exactly once, left-to-right.
            // A new SSA vector contains only the callee's slots and is discarded
            // at the call result. Branch-local emission occurs in the current CFG block.
            let mut frame = Vec::with_capacity(args.len() + initializers.len());
            for arg in args {
                frame.push(emit(arg, b, params, locals, errors));
            }
            for init in initializers {
                let value = emit(init, b, &[], &frame, errors);
                frame.push(value);
            }
            emit(body, b, &[], &frame, errors)
        }
        InlineTask(body) => emit(body, b, params, locals, errors),
        Sequence(prefix, last) => {
            for part in prefix {
                emit(part, b, params, locals, errors);
            }
            emit(last, b, params, locals, errors)
        }
        Seq(parts) => {
            // The source/host-AST gate guarantees a nonempty bounded list.
            // Emit every member: checked errors on discarded entries MUST
            // still branch before the result-store instruction.
            let (first, rest) = parts.split_first().expect("validated nonempty seq");
            let mut value = emit(first, b, params, locals, errors);
            for part in rest {
                value = emit(part, b, params, locals, errors);
            }
            value
        }
        ShiftRight8(x) => {
            // Signed Cranelift arithmetic shift of i64, preserving sign.
            // The parser admits only the constant 8, matching ODB1 byte
            // extraction for 0..i31::MAX without float narrowing.
            let input = emit(x, b, params, locals, errors);
            let eight = b.ins().iconst(types::I64, 8);
            b.ins().sshr(input, eight)
        }
        BitXor(x, y) | BitAnd(x, y) | BitOr(x, y) => {
            // Native signed-i64 bitwise primitives, not host-side emulation.
            // Emit and check each operand in lexical order before the bitwise
            // instruction; a checked arithmetic failure cannot be skipped.
            let lhs = emit(x, b, params, locals, errors);
            let rhs = emit(y, b, params, locals, errors);
            match e {
                BitXor(_, _) => b.ins().bxor(lhs, rhs),
                BitAnd(_, _) => b.ins().band(lhs, rhs),
                BitOr(_, _) => b.ins().bor(lhs, rhs),
                _ => unreachable!(),
            }
        }
        I32Checked(x) => {
            let value = emit(x, b, params, locals, errors);
            let below = b
                .ins()
                .icmp_imm_s(IntCC::SignedLessThan, value, i32::MIN as i64);
            let above = b
                .ins()
                .icmp_imm_s(IntCC::SignedGreaterThan, value, i32::MAX as i64);
            let invalid = b.ins().bor(below, above);
            let valid = b.create_block();
            b.ins().brif(invalid, i32_error, &[], valid, &[]);
            b.switch_to_block(valid);
            b.seal_block(valid);
            value
        }
        RefinementCast(x, lo, hi) => {
            let value = emit(x, b, params, locals, errors);
            let below = b.ins().icmp_imm_s(IntCC::SignedLessThan, value, *lo);
            let above = b.ins().icmp_imm_s(IntCC::SignedGreaterThan, value, *hi);
            let invalid = b.ins().bor(below, above);
            let valid = b.create_block();
            b.ins().brif(invalid, refinement_error, &[], valid, &[]);
            b.switch_to_block(valid);
            b.seal_block(valid);
            value
        }
        Div(x, y) => {
            let left = emit(x, b, params, locals, errors);
            let right = emit(y, b, params, locals, errors);
            let zero = b.ins().icmp_imm_s(IntCC::Equal, right, 0);
            let nonzero = b.create_block();
            b.ins().brif(zero, divisor_error, &[], nonzero, &[]);
            b.switch_to_block(nonzero);
            b.seal_block(nonzero);
            if unsigned {
                return b.ins().udiv(left, right);
            }
            let minimum = b.ins().icmp_imm_s(IntCC::Equal, left, i64::MIN);
            let minus_one = b.ins().icmp_imm_s(IntCC::Equal, right, -1);
            let overflow = b.ins().band(minimum, minus_one);
            let safe = b.create_block();
            b.ins().brif(overflow, error, &[], safe, &[]);
            b.switch_to_block(safe);
            b.seal_block(safe);
            b.ins().sdiv(left, right)
        }
        NamedRem(x, y) | NamedMod(x, y) => {
            let left = emit(x, b, params, locals, errors);
            let right = emit(y, b, params, locals, errors);
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
            let base = emit(x, b, params, locals, errors);
            let exponent = emit(y, b, params, locals, errors);
            checked_pow_values(base, exponent, b, error, negative_exponent_error)
        }
        _ => {
            let (value, overflow) = match e {
                Neg(x) => {
                    let rhs = emit(x, b, params, locals, errors);
                    let zero = b.ins().iconst(types::I64, 0);
                    b.ins().ssub_overflow(zero, rhs)
                }
                Add(x, y) => {
                    let lhs = emit(x, b, params, locals, errors);
                    let rhs = emit(y, b, params, locals, errors);
                    if unsigned {
                        b.ins().uadd_overflow(lhs, rhs)
                    } else {
                        b.ins().sadd_overflow(lhs, rhs)
                    }
                }
                Sub(x, y) => {
                    let lhs = emit(x, b, params, locals, errors);
                    let rhs = emit(y, b, params, locals, errors);
                    if unsigned {
                        b.ins().usub_overflow(lhs, rhs)
                    } else {
                        b.ins().ssub_overflow(lhs, rhs)
                    }
                }
                Mul(x, y) => {
                    let lhs = emit(x, b, params, locals, errors);
                    let rhs = emit(y, b, params, locals, errors);
                    if unsigned {
                        b.ins().umul_overflow(lhs, rhs)
                    } else {
                        b.ins().smul_overflow(lhs, rhs)
                    }
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

// Traced return scopes cannot currently surround runtime val initialization.
// Revalidate host-constructed trace sequences before creating native artifacts.
fn validate_trace_steps(source: &Program) -> Result<(), String> {
    if source.trace_steps.len() > 128
        || (!source.trace_steps.is_empty() && !source.locals.is_empty())
    {
        return Err(
            "E_TELEMETRY_SCOPE: unsupported scope composition or scope budget exceeded".into(),
        );
    }
    let mut depth = 0usize;
    for step in &source.trace_steps {
        match step {
            TraceStep::Begin(name) => {
                if depth >= 16
                    || name.is_empty()
                    || name.len() > 256
                    || name.chars().any(char::is_control)
                    || name.contains('\\')
                {
                    return Err("E_TELEMETRY_SCOPE: invalid scope name or depth".into());
                }
                depth += 1;
            }
            TraceStep::End => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("E_TELEMETRY_SCOPE: unmatched close")?;
            }
        }
    }
    Ok(())
}

fn prepare_ir(
    source: &Program,
    module: &mut impl Module,
    linkage: Linkage,
    unsigned: bool,
) -> Result<(cranelift_codegen::Context, cranelift_module::FuncId), String> {
    prepare_ir_inner(source,module,linkage,unsigned,None)
}
fn prepare_ir_inner(
    source: &Program,
    module: &mut impl Module,
    linkage: Linkage,
    unsigned: bool,
    placement: Option<&crate::frontend::point_placement::Plan>,
) -> Result<(cranelift_codegen::Context, cranelift_module::FuncId), String> {
    if let Some(plan)=placement {plan.verify()?;}
    validate_guest_program(source)?;
    validate_trace_steps(source)?;
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
        .declare_function(
            if unsigned {
                "atto_u64_scalar_entry"
            } else {
                "oreslang_scalar_entry"
            },
            linkage,
            &sig,
        )
        .map_err(|e| e.to_string())?;
    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let ptr = module.target_config().pointer_type();
    let trace = if source.trace_steps.is_empty() {
        None
    } else {
        let mut begin = module.make_signature();
        begin
            .params
            .extend([AbiParam::new(ptr), AbiParam::new(types::I64)]);
        begin.returns.push(AbiParam::new(types::I64));
        let mut end = module.make_signature();
        end.params.push(AbiParam::new(types::I64));
        let begin = module
            .declare_function(telemetry::BEGIN, Linkage::Import, &begin)
            .map_err(|e| e.to_string())?;
        let end = module
            .declare_function(telemetry::END, Linkage::Import, &end)
            .map_err(|e| e.to_string())?;
        let steps = source
            .trace_steps
            .iter()
            .map(|step| match step {
                TraceStep::Begin(operation) => {
                    let id = module
                        .declare_anonymous_data(false, false)
                        .map_err(|e| e.to_string())?;
                    let mut data = DataDescription::new();
                    data.define(operation.as_bytes().into());
                    module.define_data(id, &data).map_err(|e| e.to_string())?;
                    Ok(Some((
                        module.declare_data_in_func(id, &mut ctx.func),
                        operation.len(),
                    )))
                }
                TraceStep::End => Ok(None),
            })
            .collect::<Result<Vec<Option<(GlobalValue, usize)>>, String>>()?;
        Some((
            module.declare_func_in_func(begin, &mut ctx.func),
            module.declare_func_in_func(end, &mut ctx.func),
            steps,
        ))
    };
    let mut fbctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
    let entry = b.create_block();
    let failure = b.create_block();
    let divisor_error = b.create_block();
    let negative_exponent_error = b.create_block();
    let refinement_error = b.create_block();
    let i32_error = b.create_block();
    let errors = ScalarErrorBlocks {
        error: failure,
        divisor_error,
        negative_exponent_error,
        refinement_error,
        i32_error,
        unsigned,
    };
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let p = b.block_params(entry).to_vec();
    let mut active = Vec::new();
    if let Some((begin, end, steps)) = &trace {
        for step in steps {
            match step {
                Some((name, len)) => {
                    let ptr = b.ins().symbol_value(ptr, *name);
                    let len = b.ins().iconst(types::I64, *len as i64);
                    let call = b.ins().call(*begin, &[ptr, len]);
                    active.push(b.inst_results(call)[0]);
                }
                None => {
                    let token = active.pop().ok_or("missing lexical span for close")?;
                    b.ins().call(*end, &[token]);
                }
            }
        }
    }
    let cleanup = (!active.is_empty()).then(|| {
        let block = b.create_block();
        b.append_block_param(block, types::I32);
        block
    });
    let mut locals = Vec::with_capacity(source.locals.len());
    for initializer in &source.locals {
        // A runtime val initializer runs once in lexical order, even if it
        // is unused; checked traps branch away before any output store.
        let value = emit(initializer, &mut b, &p[..4], &locals, errors);
        locals.push(value);
    }
    let value = if let Some(plan)=placement {
        use crate::frontend::point_placement::Step;
        use cranelift_codegen::ir::{StackSlotData,StackSlotKind};
        let storage=b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot,16,3));
        let mut copies=Vec::new();
        for step in &plan.steps {
            match *step {
                Step::Construct(x,y)=>{
                    let a=b.ins().iconst(types::I64,x);let z=b.ins().iconst(types::I64,y);
                    b.ins().stack_store(ptr,a,storage,0);b.ins().stack_store(ptr,z,storage,8);
                }
                Step::CopyField(f)=>copies.push(b.ins().stack_load(ptr,types::I64,storage,i32::from(f)*8)),
                Step::Destroy|Step::Free=>{
                    let zero=b.ins().iconst(types::I64,0);
                    b.ins().stack_store(ptr,zero,storage,0);b.ins().stack_store(ptr,zero,storage,8);
                }
                Step::Bind|Step::Detach=>{}
            }
        }
        let a=copies[plan.result[0]];
        if plan.result.len()==2 {b.ins().iadd(a,copies[plan.result[1]])}else{a}
    } else {emit(&source.result, &mut b, &p[..4], &locals, errors)};
    b.ins().store(MemFlagsData::new(), value, p[4], 0);
    return_status(&mut b, 0, cleanup);
    for (block, status) in [
        (failure, 1),
        (divisor_error, 2),
        (negative_exponent_error, 3),
        (refinement_error, 7),
        (i32_error, 8),
    ] {
        b.switch_to_block(block);
        return_status(&mut b, status, cleanup);
    }
    if let Some(block) = cleanup {
        b.switch_to_block(block);
        let status = b.block_params(block)[0];
        let end = trace.expect("live spans have an end hook").1;
        for token in active.into_iter().rev() {
            b.ins().call(end, &[token]);
        }
        b.ins().return_(&[status]);
    }
    b.seal_all_blocks();
    b.finalize(module.target_config());
    // Unconditional SSA, CFG, type and ABI verification before native emission.
    // This must not depend on Cranelift's optional enable_verifier setting.
    validate_ir(&ctx.func, module.isa())?;
    Ok((ctx, id))
}

fn return_status(b: &mut FunctionBuilder, status: i64, cleanup: Option<Block>) {
    let status = b.ins().iconst(types::I32, status);
    if let Some(block) = cleanup {
        b.ins().jump(block, &[BlockArg::Value(status)]);
    } else {
        b.ins().return_(&[status]);
    }
}

fn jit_module() -> Result<JITModule, String> {
    let mut builder = JITBuilder::new(default_libcall_names()).map_err(|e| e.to_string())?;
    builder.symbol(telemetry::BEGIN, telemetry::begin as *const u8);
    builder.symbol(telemetry::END, telemetry::end as *const u8);
    Ok(JITModule::new(builder))
}

// Constructs and verifies the same CLIF as the native compiler without
// define_function, finalize_definitions, or executable-code allocation.
pub fn render_ir(source: &Program) -> Result<String, String> {
    let mut module = jit_module()?;
    let (ctx, _) = prepare_ir(source, &mut module, Linkage::Local, false)?;
    Ok(ctx.func.display().to_string())
}

pub fn compile(source: &Program) -> Result<Compiled, String> {
    let mut module = jit_module()?;
    let (mut ctx, id) = prepare_ir(source, &mut module, Linkage::Local, false)?;
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

/// Emit a host-target relocatable object for the verified scalar entry ABI.
/// Native linking uses a caller-supplied host. This path owns no JIT module.
pub fn emit_object(source: &Program) -> Result<Vec<u8>, String> {
    emit_scalar_object(source, false)
}

pub(crate) fn emit_unsigned_object(
    source: &crate::frontend::UnsignedProgram,
) -> Result<Vec<u8>, String> {
    emit_scalar_object(source.inner(), true)
}

fn emit_scalar_object(source: &Program, unsigned: bool) -> Result<Vec<u8>, String> {
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "atto_scalar", default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let (mut ctx, id) = prepare_ir(source, &mut module, Linkage::Export, unsigned)?;
    module
        .define_function(id, &mut ctx)
        .map_err(|e| e.to_string())?;
    module.finish().emit().map_err(|e| e.to_string())
}

impl Compiled {
    pub fn run_traced(&self, args: &[i64]) -> (Result<i64, String>, Vec<telemetry::Span>) {
        telemetry::record(|| self.run(args))
    }

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
        } else if status == 7 {
            Err("E_REFINEMENT_VIOLATION".into())
        } else if status == 8 {
            Err("E_I32_OVERFLOW".into())
        } else {
            Err("invalid generated native status".into())
        }
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;

    #[test]
    fn traced_machine_code_retires_nested_scopes_on_return_and_checked_error() {
        let source = crate::frontend::parse(
            r#"
            pub fnc main(int input): int {
                using telemetry.span("finished") as finished {}
                using telemetry.span("outer") as outer {
                    using telemetry.span("inner") as inner { return input + 1; }
                }
                return 0;
            }
        "#,
        )
        .unwrap();
        let guest = compile(&source).unwrap();
        assert_eq!(guest.run(&[41]), Ok(42));
        for (input, expected) in [
            (41, Ok(42)),
            (i64::MAX, Err("i64 arithmetic overflow".into())),
        ] {
            let (outcome, spans) = guest.run_traced(&[input]);
            assert_eq!(outcome, expected);
            assert_eq!(
                spans
                    .iter()
                    .map(|span| span.operation.as_str())
                    .collect::<Vec<_>>(),
                ["finished", "inner", "outer"]
            );
            assert_eq!(spans[0].parent, 0);
            assert_eq!(spans[1].parent, spans[2].id);
            assert_eq!(spans[2].parent, 0);
        }
        assert_eq!(guest.run(&[41]), Ok(42));
    }

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

    #[test]
    fn checked_sequence_jit_matches_reference_and_rejects_forged_ast() {
        for expression in [
            "seq(42)",
            "seq(1,2,42)",
            "(> 1,2,42)",
            "seq(1, (> 2,42))",
            "seq(1,Integer.bitAnd(7,3),42)",
            "seq(1,2)+40",
        ] {
            let source = format!("fnc main():int{{return {expression};}}");
            let parsed = crate::frontend::parse(&source).expect("valid sequence");
            let checked_ir = render_ir(&parsed).expect("verified native IR");
            let compiled = compile(&parsed).expect("real Cranelift JIT");
            assert_eq!(checked_ir, compiled.clif, "IR/JIT drift for {source}");
            assert_eq!(compiled.run(&[]), Ok(42), "{source}");
        }
        for (expression, error) in [
            ("seq(9223372036854775807+1,42)", "i64 arithmetic overflow"),
            ("(> 9223372036854775807+1,42)", "i64 arithmetic overflow"),
            ("seq(Integer.rem(8,0),42)", "integer divisor zero"),
            ("(> Integer.mod(8,0),42)", "integer divisor zero"),
        ] {
            let source = format!("fnc main():int{{return {expression};}}");
            let parsed = crate::frontend::parse(&source).expect("valid but trapping sequence");
            let compiled = compile(&parsed).expect("checked guest IR");
            assert_eq!(compiled.run(&[]), Err(error.to_owned()), "{source}");
        }
        for result in [Expr::Seq(vec![]), Expr::Seq(vec![Expr::Literal(42); 17])] {
            let forged = Program {
                trace_steps: Vec::new(),
                parameters: vec![],
                locals: vec![],
                result,
            };
            assert!(
                render_ir(&forged).is_err(),
                "invalid AST admitted to CLIF IR"
            );
            assert!(compile(&forged).is_err(), "invalid AST reached JIT");
        }
    }

    #[test]
    fn host_constructed_forward_or_forged_references_fail_before_clif() {
        let self_reference = Program {
            trace_steps: Vec::new(),
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
            trace_steps: Vec::new(),
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
    fn forged_native_abi_arity_and_balanced_node_budget_fail_before_jit() {
        // Port the unique source-AST security cases from overlapping #75,
        // without importing its obsolete runtime type-first local semantics.
        let excess = Program {
            trace_steps: Vec::new(),
            parameters: (0..5).map(|index| format!("p{index}")).collect(),
            locals: Vec::new(),
            result: Expr::Literal(42),
        };
        for outcome in [
            render_ir(&excess).err(),
            compile(&excess).err().map(|e| e.to_string()),
        ] {
            assert!(
                outcome
                    .expect("five parameter ABI must be rejected")
                    .contains("parameter count exceeds four")
            );
        }

        fn balanced(depth: usize) -> Expr {
            if depth == 0 {
                Expr::Literal(1)
            } else {
                Expr::Add(Box::new(balanced(depth - 1)), Box::new(balanced(depth - 1)))
            }
        }
        let oversized = Program {
            trace_steps: Vec::new(),
            parameters: Vec::new(),
            locals: Vec::new(),
            result: balanced(12),
        };
        assert!(
            render_ir(&oversized)
                .unwrap_err()
                .contains("AST node budget exceeded")
        );
        assert!(
            compile(&oversized)
                .err()
                .expect("forged balanced AST must not emit code")
                .contains("AST node budget exceeded")
        );
    }

    #[test]
    fn unused_runtime_val_overflow_preserves_native_output_sentinel() {
        let source = crate::frontend::parse("fnc main(x:int):int{val unused=x+1;return 42;}")
            .expect("runtime val may depend on a parameter");
        let compiled = compile(&source).expect("native verified CLIF/JIT");
        type Entry = unsafe extern "C" fn(i64, i64, i64, i64, *mut i64) -> i32;
        let function: Entry =
            unsafe { std::mem::transmute(compiled.module.get_finalized_function(compiled.id)) };
        let mut output = 0x1122_3344_5566_7788_i64;
        let status = unsafe { function(i64::MAX, 0, 0, 0, &mut output) };
        assert_eq!(status, 1, "overflow must return a native error status");
        assert_eq!(
            output, 0x1122_3344_5566_7788_i64,
            "failed unused initializer must not publish a guest value"
        );
    }

    #[test]
    fn direct_ast_limits_reject_before_jit() {
        let too_many = Program {
            trace_steps: Vec::new(),
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
            trace_steps: Vec::new(),
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
                trace_steps: Vec::new(),
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

#[cfg(test)]
mod scoped_helper_frame_tests {
    use super::*;
    #[test]
    fn scoped_frames_do_not_admit_caller_or_forward_slot_references() {
        let valid = Expr::ScopedCall(
            vec![Expr::Local(0)],
            vec![Expr::Local(0)],
            Box::new(Expr::Local(1)),
        );
        assert!(validate_guest_expr(&valid, 1, 0, &mut 0).is_ok());
        for bad in [
            Expr::ScopedCall(vec![], vec![], Box::new(Expr::Local(0))),
            Expr::ScopedCall(
                vec![Expr::Literal(1)],
                vec![Expr::Local(1)],
                Box::new(Expr::Local(0)),
            ),
            Expr::ScopedCall(vec![], vec![], Box::new(Expr::Param(0))),
            Expr::ScopedCall(vec![Expr::Literal(1); 9], vec![], Box::new(Expr::Local(0))),
        ] {
            assert!(validate_guest_expr(&bad, 2, 1, &mut 0).is_err());
        }
    }
}

// Native object only. No JIT allocation or guest storage capability.
pub fn emit_durable_counter_object(source: &str) -> Result<Vec<u8>, String> {
    let descriptor = crate::frontend::durable_counter_descriptor(source)?;
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "atto_durable_counter", default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let id = module
        .declare_data("atto_durable_descriptor_v1", Linkage::Export, false, false)
        .map_err(|e| e.to_string())?;
    let mut data = DataDescription::new();
    data.define(descriptor.into_boxed_slice());
    module.define_data(id, &data).map_err(|e| e.to_string())?;
    let mut sig = module.make_signature();
    for _ in 0..3 {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    let id = module
        .declare_function("atto_durable_transition_v1", Linkage::Export, &sig)
        .map_err(|e| e.to_string())?;
    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fbctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
    let entry = b.create_block();
    let dispatch = b.create_block();
    let read = b.create_block();
    let increment = b.create_block();
    let bounds = b.create_block();
    let add = b.create_block();
    let reject = b.create_block();
    let read_ok = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let args = b.block_params(entry).to_vec();
    let (s, op, d) = (args[0], args[1], args[2]);
    let low = b.ins().icmp_imm_s(IntCC::SignedLessThan, s, 0);
    let high = b
        .ins()
        .icmp_imm_s(IntCC::SignedGreaterThan, s, 1_000_000_000);
    let bad = b.ins().bor(low, high);
    b.ins().brif(bad, reject, &[], dispatch, &[]);
    b.switch_to_block(dispatch);
    let is_read = b.ins().icmp_imm_s(IntCC::Equal, op, 2);
    b.ins().brif(is_read, read, &[], increment, &[]);
    b.switch_to_block(read);
    let zero = b.ins().icmp_imm_s(IntCC::Equal, d, 0);
    b.ins().brif(zero, read_ok, &[], reject, &[]);
    b.switch_to_block(read_ok);
    b.ins().return_(&[s]);
    b.switch_to_block(increment);
    let wrong = b.ins().icmp_imm_s(IntCC::NotEqual, op, 1);
    let low = b.ins().icmp_imm_s(IntCC::SignedLessThanOrEqual, d, 0);
    let high = b
        .ins()
        .icmp_imm_s(IntCC::SignedGreaterThan, d, 1_000_000_000);
    let bad = b.ins().bor(low, high);
    let bad = b.ins().bor(bad, wrong);
    b.ins().brif(bad, reject, &[], bounds, &[]);
    b.switch_to_block(bounds);
    let max = b.ins().iconst(types::I64, 1_000_000_000);
    let remaining = b.ins().isub(max, s);
    let overflow = b.ins().icmp(IntCC::SignedGreaterThan, d, remaining);
    b.ins().brif(overflow, reject, &[], add, &[]);
    b.switch_to_block(add);
    let next = b.ins().iadd(s, d);
    b.ins().return_(&[next]);
    b.switch_to_block(reject);
    let denied = b.ins().iconst(types::I64, -1);
    b.ins().return_(&[denied]);
    b.seal_all_blocks();
    b.finalize(module.target_config());
    validate_ir(&ctx.func, module.isa())?;
    module
        .define_function(id, &mut ctx)
        .map_err(|e| e.to_string())?;
    module.finish().emit().map_err(|e| e.to_string())
}

// Native schema-specialized codecs. Every input byte/value is staged in SSA
// before the first output write. ABI requires trusted, valid, quiescent
// caller-owned ranges, including non-null dummy pointers for empty schemas.
pub fn emit_durable_scalar_object(source: &str) -> Result<Vec<u8>, String> {
    let (initial, tags) = crate::frontend::durable_scalar_schema(source)?;
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "atto_durable_scalar", default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    if module.target_config().pointer_type() != types::I64 {
        return Err("E_DURABLE_SCALAR_SCHEMA_V1: 64-bit native target required".into());
    }
    let size = initial.len() as i64;
    let count = tags.len() as i64;
    fn hash(b: &mut FunctionBuilder, bytes: &[Value]) -> Value {
        let mut h = b.ins().iconst(types::I64, 0xcbf29ce484222325_u64 as i64);
        let prime = b.ins().iconst(types::I64, 0x100000001b3);
        for &byte in bytes {
            let xor = b.ins().bxor(h, byte);
            h = b.ins().imul(xor, prime);
        }
        h
    }
    fn byte_at(b: &mut FunctionBuilder, value: Value, i: usize) -> Value {
        let shift = b.ins().iconst(types::I64, (i * 8) as i64);
        let shifted = b.ins().ushr(value, shift);
        b.ins().band_imm_u(shifted, 255)
    }
    fn word_at(b: &mut FunctionBuilder, bytes: &[Value]) -> Value {
        let mut word = b.ins().iconst(types::I64, 0);
        for (i, &byte) in bytes.iter().enumerate() {
            let shift = b.ins().iconst(types::I64, (i * 8) as i64);
            let part = b.ins().ishl(byte, shift);
            word = b.ins().bor(word, part);
        }
        word
    }
    fn require(b: &mut FunctionBuilder, ok: Value, reject: Block) {
        let next = b.create_block();
        b.ins().brif(ok, next, &[], reject, &[]);
        b.switch_to_block(next);
    }
    fn output_bytes(b: &mut FunctionBuilder, dst: Value, bytes: &[Value]) {
        for (i, &byte) in bytes.iter().enumerate() {
            let v = b.ins().ireduce(types::I8, byte);
            b.ins().store(MemFlagsData::new(), v, dst, i as i32);
        }
    }
    for (name, arity, mode) in [
        ("atto_scalar_size_v1", 0, 0),
        ("atto_scalar_fields_v1", 0, 1),
        ("atto_scalar_initial_v1", 2, 2),
        ("atto_scalar_encode_v1", 4, 3),
        ("atto_scalar_decode_v1", 4, 4),
    ] {
        let mut sig = module.make_signature();
        for _ in 0..arity {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let mut fbctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let args = b.block_params(entry).to_vec();
        if mode < 2 {
            let v = b
                .ins()
                .iconst(types::I64, if mode == 0 { size } else { count });
            b.ins().return_(&[v]);
        } else {
            let reject = b.create_block();
            let dst = if mode == 2 { args[0] } else { args[2] };
            let cap = if mode == 2 { args[1] } else { args[3] };
            let needed = if mode == 4 { count } else { size };
            let nonnull = b.ins().icmp_imm_s(IntCC::NotEqual, dst, 0);
            require(&mut b, nonnull, reject);
            let enough = b
                .ins()
                .icmp_imm_s(IntCC::SignedGreaterThanOrEqual, cap, needed);
            require(&mut b, enough, reject);
            if mode == 2 {
                let bytes: Vec<Value> = initial
                    .iter()
                    .map(|&x| b.ins().iconst(types::I64, i64::from(x)))
                    .collect();
                output_bytes(&mut b, dst, &bytes);
            } else {
                let src = args[0];
                let nonnull = b.ins().icmp_imm_s(IntCC::NotEqual, src, 0);
                require(&mut b, nonnull, reject);
                let len =
                    b.ins()
                        .icmp_imm_s(IntCC::Equal, args[1], if mode == 3 { count } else { size });
                require(&mut b, len, reject);
                if mode == 3 {
                    let mut bytes: Vec<Value> = initial[..19]
                        .iter()
                        .map(|&x| b.ins().iconst(types::I64, i64::from(x)))
                        .collect();
                    for (i, &tag) in tags.iter().enumerate() {
                        let value =
                            b.ins()
                                .load(types::I64, MemFlagsData::new(), src, (i * 8) as i32);
                        bytes.push(b.ins().iconst(types::I64, i64::from(tag)));
                        if tag == 1 {
                            for j in 0..8 {
                                bytes.push(byte_at(&mut b, value, j));
                            }
                        } else {
                            let ok = b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, value, 1);
                            require(&mut b, ok, reject);
                            bytes.push(value);
                        }
                    }
                    let checksum = hash(&mut b, &bytes);
                    for j in 0..8 {
                        bytes.push(byte_at(&mut b, checksum, j));
                    }
                    output_bytes(&mut b, dst, &bytes);
                } else {
                    let mut bytes = Vec::with_capacity(size as usize);
                    for i in 0..size {
                        let byte = b.ins().load(types::I8, MemFlagsData::new(), src, i as i32);
                        bytes.push(b.ins().uextend(types::I64, byte));
                    }
                    let end = bytes.len() - 8;
                    let checksum = hash(&mut b, &bytes[..end]);
                    let stored = word_at(&mut b, &bytes[end..]);
                    let ok = b.ins().icmp(IntCC::Equal, stored, checksum);
                    require(&mut b, ok, reject);
                    for (i, &x) in initial[..19].iter().enumerate() {
                        let ok = b.ins().icmp_imm_s(IntCC::Equal, bytes[i], i64::from(x));
                        require(&mut b, ok, reject);
                    }
                    let mut pos = 19;
                    let mut values = Vec::with_capacity(tags.len());
                    for &tag in &tags {
                        let ok = b.ins().icmp_imm_s(IntCC::Equal, bytes[pos], i64::from(tag));
                        require(&mut b, ok, reject);
                        pos += 1;
                        if tag == 1 {
                            values.push(word_at(&mut b, &bytes[pos..pos + 8]));
                            pos += 8;
                        } else {
                            let value = bytes[pos];
                            let ok = b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, value, 1);
                            require(&mut b, ok, reject);
                            values.push(value);
                            pos += 1;
                        }
                    }
                    for (i, value) in values.into_iter().enumerate() {
                        b.ins()
                            .store(MemFlagsData::new(), value, dst, (i * 8) as i32);
                    }
                }
            }
            let v = b.ins().iconst(types::I64, needed);
            b.ins().return_(&[v]);
            b.switch_to_block(reject);
            let v = b.ins().iconst(types::I64, -1);
            b.ins().return_(&[v]);
        }
        b.seal_all_blocks();
        b.finalize(module.target_config());
        validate_ir(&ctx.func, module.isa())?;
        module
            .define_function(id, &mut ctx)
            .map_err(|e| e.to_string())?;
    }
    module.finish().emit().map_err(|e| e.to_string())
}

pub(crate) fn placement_command(plan:&crate::frontend::point_placement::Plan,mode:&str)->Result<(),String>{
    use std::io::Write;
    plan.verify()?;
    let source=Program{parameters:vec![],locals:vec![],result:Expr::Literal(0),trace_steps:vec![]};
    match mode {
        "check"=>Ok(()),
        "ir"=>{
            let mut module=jit_module()?;
            let (ctx,_)=prepare_ir_inner(&source,&mut module,Linkage::Local,false,Some(plan))?;
            print!("{}",ctx.func.display());Ok(())
        }
        "run"=>{
            let mut module=jit_module()?;
            let (mut ctx,id)=prepare_ir_inner(&source,&mut module,Linkage::Local,false,Some(plan))?;
            #[cfg(test)] let clif=ctx.func.display().to_string();
            module.define_function(id,&mut ctx).map_err(|e|e.to_string())?;
            module.clear_context(&mut ctx);
            module.finalize_definitions().map_err(|e|e.to_string())?;
            let compiled=Compiled{module,id,arity:0,#[cfg(test)]clif};
            println!("{}",compiled.run(&[])?);Ok(())
        }
        "emit-object"=>{
            let mut flags=settings::builder();
            flags.set("is_pic","true").map_err(|e|e.to_string())?;
            let isa=cranelift_native::builder().map_err(|e|e.to_string())?.finish(settings::Flags::new(flags)).map_err(|e|e.to_string())?;
            let builder=ObjectBuilder::new(isa,"atto_placement",default_libcall_names()).map_err(|e|e.to_string())?;
            let mut module=ObjectModule::new(builder);
            let (mut ctx,id)=prepare_ir_inner(&source,&mut module,Linkage::Export,false,Some(plan))?;
            module.define_function(id,&mut ctx).map_err(|e|e.to_string())?;
            let bytes=module.finish().emit().map_err(|e|e.to_string())?;
            std::io::stdout().lock().write_all(&bytes).map_err(|e|e.to_string())
        }
        _=>Err("E_POINTER_PLACEMENT: unsupported invocation mode".into()),
    }
}
