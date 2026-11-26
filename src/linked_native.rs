//! Actual per-function Cranelift JIT code generation for trusted, bounded
//! linked i64 modules. Each selected helper is a separate native function.
//!
//! Helper ABI: (i64 arguments...) -> (i64 value, i32 status).
//! Entry ABI:  (i64, i64, i64, i64, *mut i64) -> i32 status.
//! A nonzero status propagates unchanged; the entry output is stored only
//! after all checked arguments, helpers and local initializers succeed.
//! This JIT MUST NEVER be described as a hostile-tenant memory sandbox.
use crate::backend::Compiled;
use crate::functional_pipelines::{Fn as ScalarFn, Raw, ScalarType};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::instructions::BlockArg;
use cranelift_codegen::ir::{AbiParam, Block, FuncRef, InstBuilder, MemFlagsData, Value, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
use std::collections::HashMap;

const MAX_FUNCTIONS: usize = 16;
const MAX_CALL_DEPTH: usize = 16;
const MAX_EXPR_DEPTH: usize = 32;
const MAX_EXPR_NODES: usize = 4096;
const MAX_LOCALS: usize = 4;

fn inspect_expression(
    expr: &Raw,
    scope: &HashMap<String, ScalarType>,
    functions: &HashMap<String, usize>,
    program: &[ScalarFn],
    dependencies: &mut Vec<String>,
    nodes: &mut usize,
    depth: usize,
) -> Result<ScalarType, String> {
    if depth > MAX_EXPR_DEPTH {
        return Err("E_LINK_EXPR_DEPTH".into());
    }
    *nodes += 1;
    if *nodes > MAX_EXPR_NODES {
        return Err("E_LINK_EXPR_BUDGET".into());
    }
    use ScalarType::{Bool, Int};
    let mut child =
        |e: &Raw| inspect_expression(e, scope, functions, program, dependencies, nodes, depth + 1);
    match expr {
        Raw::Tuple(_) => Err("E_LINK_UNSUPPORTED_TUPLE_VALUE".into()),
        Raw::TuplePattern(values, bindings, body) => {
            if values.len() != bindings.len() || values.len() > 8 {
                return Err("E_LINK_PATTERN_ARITY".into());
            }
            // All root elements are typed in the outer scope, including discards.
            for value in values {
                if child(value)? != Int {
                    return Err("E_LINK_PATTERN_ELEMENT_TYPE".into());
                }
            }
            let mut nested = scope.clone();
            let mut names = std::collections::HashSet::new();
            for name in bindings.iter().flatten() {
                if matches!(name.as_str(), "_" | "self" | "arguments") || !names.insert(name) {
                    return Err("E_LINK_PATTERN_DUP_BINDING".into());
                }
                nested.insert(name.clone(), Int);
            }
            let ty = inspect_expression(
                body,
                &nested,
                functions,
                program,
                dependencies,
                nodes,
                depth + 1,
            )?;
            if ty != Int {
                return Err("E_LINK_PATTERN_RESULT_TYPE".into());
            }
            Ok(Int)
        }
        Raw::Literal(_) => Ok(Int),
        Raw::Bool(_) => Ok(Bool),
        Raw::Name(name) => scope
            .get(name)
            .copied()
            .ok_or_else(|| "E_LINK_UNBOUND_IDENTIFIER".into()),
        Raw::Add(a, b) | Raw::Sub(a, b) | Raw::Mul(a, b) => {
            if child(a)? != Int || child(b)? != Int {
                return Err("E_LINK_INTEGER_OPERANDS".into());
            }
            Ok(Int)
        }
        Raw::Eq(a, b) | Raw::Ne(a, b) => {
            let lhs = child(a)?;
            let rhs = child(b)?;
            if lhs != rhs {
                return Err("E_LINK_EQUALITY_TYPES".into());
            }
            Ok(Bool)
        }
        Raw::And(a, b) | Raw::Or(a, b) | Raw::Xor(a, b) => {
            if child(a)? != Bool || child(b)? != Bool {
                return Err("E_LINK_BOOLEAN_OPERANDS".into());
            }
            Ok(Bool)
        }
        Raw::Not(a) => {
            if child(a)? != Bool {
                return Err("E_LINK_BOOLEAN_OPERANDS".into());
            }
            Ok(Bool)
        }
        Raw::If(condition, yes, no) => {
            if child(condition)? != Bool {
                return Err("E_LINK_BOOLEAN_CONDITION".into());
            }
            let y = child(yes)?;
            let n = child(no)?;
            if y != n {
                return Err("E_LINK_BRANCH_TYPES".into());
            }
            Ok(y)
        }
        Raw::Call(target, args) => {
            let index = functions.get(target).ok_or("E_LINK_UNBOUND_CALLEE")?;
            let callee = &program[*index];
            if target == "main" || args.len() != callee.params.len() {
                return Err("E_LINK_CALL_ARITY_OR_ENTRY".into());
            }
            for (arg, ty) in args.iter().zip(&callee.param_types) {
                if child(arg)? != *ty {
                    return Err("E_LINK_ARGUMENT_TYPE".into());
                }
            }
            dependencies.push(target.clone());
            Ok(callee.return_type)
        }
    }
}

fn cycle_visit(
    index: usize,
    names: &HashMap<String, usize>,
    edges: &[Vec<String>],
    states: &mut [u8],
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_CALL_DEPTH {
        return Err("E_LINK_CALL_DEPTH".into());
    }
    if states[index] == 1 {
        return Err("E_LINK_CALL_CYCLE".into());
    }
    if states[index] == 2 {
        return Ok(());
    }
    states[index] = 1;
    for target in &edges[index] {
        let other = names[target];
        cycle_visit(other, names, edges, states, depth + 1)?;
    }
    states[index] = 2;
    Ok(())
}

/// Complete graph validation precedes IR emission, including *dormant*
/// helpers; rejected sources never allocate executable code or print CLIF.
pub fn validate(program: &[ScalarFn]) -> Result<(), String> {
    if program.is_empty() || program.len() > MAX_FUNCTIONS {
        return Err("E_LINK_FUNCTION_BUDGET".into());
    }
    let mut names = HashMap::new();
    for (i, func) in program.iter().enumerate() {
        if names.insert(func.name.clone(), i).is_some() {
            return Err("E_LINK_DUP_FUNCTION".into());
        }
        if func.params.len() != func.param_types.len()
            || func.params.len() > 4
            || func.locals.len() > MAX_LOCALS
        {
            return Err("E_LINK_FUNCTION_ARITY_OR_LOCAL_BUDGET".into());
        }
    }
    let main = names.get("main").ok_or("E_LINK_MAIN_REQUIRED")?;
    if program[*main].return_type != ScalarType::Int {
        return Err("E_LINK_MAIN_TYPE".into());
    }
    if !program[*main].params.is_empty() {
        return Err("E_LINK_MAIN_ARITY".into());
    }
    let mut edges = Vec::with_capacity(program.len());
    let mut nodes = 0;
    for func in program {
        let mut scope = HashMap::new();
        for (param, ty) in func.params.iter().zip(&func.param_types) {
            if scope.insert(param.clone(), *ty).is_some() {
                return Err("E_LINK_DUP_PARAMETER".into());
            }
        }
        let mut dependencies = Vec::new();
        for (name, init) in &func.locals {
            let ty = inspect_expression(
                init,
                &scope,
                &names,
                program,
                &mut dependencies,
                &mut nodes,
                0,
            )?;
            if scope.insert(name.clone(), ty).is_some() {
                return Err("E_LINK_DUP_LOCAL".into());
            }
        }
        let actual = inspect_expression(
            &func.result,
            &scope,
            &names,
            program,
            &mut dependencies,
            &mut nodes,
            0,
        )?;
        if actual != func.return_type {
            return Err("E_LINK_RETURN_TYPE".into());
        }
        edges.push(dependencies);
    }
    let mut states = vec![0; program.len()];
    for i in 0..program.len() {
        cycle_visit(i, &names, &edges, &mut states, 0)?;
    }
    Ok(())
}

fn fail_on_overflow(builder: &mut FunctionBuilder, overflow: Value, failure: Block) {
    let status = builder.ins().iconst(types::I32, 1);
    let continuation = builder.create_block();
    builder.ins().brif(
        overflow,
        failure,
        &[BlockArg::Value(status)],
        continuation,
        &[],
    );
    builder.switch_to_block(continuation);
    builder.seal_block(continuation);
}

fn emit_expr(
    expr: &Raw,
    builder: &mut FunctionBuilder,
    scope: &HashMap<String, Value>,
    refs: &HashMap<String, FuncRef>,
    failure: Block,
    depth: usize,
) -> Result<Value, String> {
    if depth > MAX_EXPR_DEPTH {
        return Err("E_LINK_EXPR_DEPTH".into());
    }
    match expr {
        Raw::Tuple(_) => Err("E_LINK_UNSUPPORTED_TUPLE_VALUE".into()),
        Raw::TuplePattern(values, bindings, body) => {
            // Evaluate each input once, in source order, before binding any
            // names. Checked overflow/call failures branch out before body.
            let mut slots = Vec::with_capacity(values.len());
            for value in values {
                slots.push(emit_expr(value, builder, scope, refs, failure, depth + 1)?);
            }
            let mut nested = scope.clone();
            for (name, slot) in bindings.iter().zip(slots) {
                if let Some(name) = name {
                    nested.insert(name.clone(), slot);
                }
            }
            emit_expr(body, builder, &nested, refs, failure, depth + 1)
        }
        Raw::Literal(value) => Ok(builder.ins().iconst(types::I64, *value)),
        // Boolean values have a canonical internal 0/1 representation in
        // the existing private i64 helper ABI, after full nominal typing.
        Raw::Bool(value) => Ok(builder.ins().iconst(types::I64, i64::from(*value))),
        Raw::Not(inner) => {
            let v = emit_expr(inner, builder, scope, refs, failure, depth + 1)?;
            let flag = builder.ins().icmp_imm_s(IntCC::Equal, v, 0);
            Ok(builder.ins().uextend(types::I64, flag))
        }
        Raw::Eq(a, b) | Raw::Ne(a, b) | Raw::Xor(a, b) => {
            let lhs = emit_expr(a, builder, scope, refs, failure, depth + 1)?;
            let rhs = emit_expr(b, builder, scope, refs, failure, depth + 1)?;
            let cc = if matches!(expr, Raw::Eq(_, _)) {
                IntCC::Equal
            } else {
                IntCC::NotEqual
            };
            let flag = builder.ins().icmp(cc, lhs, rhs);
            Ok(builder.ins().uextend(types::I64, flag))
        }
        Raw::And(a, b) | Raw::Or(a, b) => {
            let lhs = emit_expr(a, builder, scope, refs, failure, depth + 1)?;
            let flag = builder.ins().icmp_imm_s(IntCC::NotEqual, lhs, 0);
            let rhs_block = builder.create_block();
            let merge = builder.create_block();
            builder.append_block_param(merge, types::I64);
            if matches!(expr, Raw::And(_, _)) {
                builder
                    .ins()
                    .brif(flag, rhs_block, &[], merge, &[BlockArg::Value(lhs)]);
            } else {
                builder
                    .ins()
                    .brif(flag, merge, &[BlockArg::Value(lhs)], rhs_block, &[]);
            }
            builder.switch_to_block(rhs_block);
            builder.seal_block(rhs_block);
            let rhs = emit_expr(b, builder, scope, refs, failure, depth + 1)?;
            builder.ins().jump(merge, &[BlockArg::Value(rhs)]);
            builder.switch_to_block(merge);
            builder.seal_block(merge);
            Ok(builder.block_params(merge)[0])
        }
        Raw::If(condition, yes, no) => {
            let value = emit_expr(condition, builder, scope, refs, failure, depth + 1)?;
            let flag = builder.ins().icmp_imm_s(IntCC::NotEqual, value, 0);
            let yes_block = builder.create_block();
            let no_block = builder.create_block();
            let merge = builder.create_block();
            builder.append_block_param(merge, types::I64);
            builder.ins().brif(flag, yes_block, &[], no_block, &[]);
            builder.switch_to_block(yes_block);
            builder.seal_block(yes_block);
            let y = emit_expr(yes, builder, scope, refs, failure, depth + 1)?;
            builder.ins().jump(merge, &[BlockArg::Value(y)]);
            builder.switch_to_block(no_block);
            builder.seal_block(no_block);
            let n = emit_expr(no, builder, scope, refs, failure, depth + 1)?;
            builder.ins().jump(merge, &[BlockArg::Value(n)]);
            builder.switch_to_block(merge);
            builder.seal_block(merge);
            Ok(builder.block_params(merge)[0])
        }
        Raw::Name(name) => scope
            .get(name)
            .copied()
            .ok_or_else(|| "E_LINK_UNBOUND_IDENTIFIER".into()),
        Raw::Add(a, b) | Raw::Sub(a, b) | Raw::Mul(a, b) => {
            let lhs = emit_expr(a, builder, scope, refs, failure, depth + 1)?;
            let rhs = emit_expr(b, builder, scope, refs, failure, depth + 1)?;
            let (value, overflow) = match expr {
                Raw::Add(_, _) => builder.ins().sadd_overflow(lhs, rhs),
                Raw::Sub(_, _) => builder.ins().ssub_overflow(lhs, rhs),
                Raw::Mul(_, _) => builder.ins().smul_overflow(lhs, rhs),
                _ => unreachable!(),
            };
            fail_on_overflow(builder, overflow, failure);
            Ok(value)
        }
        Raw::Call(name, arguments) => {
            let func_ref = *refs.get(name).ok_or("E_LINK_UNBOUND_CALLEE")?;
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(emit_expr(
                    argument,
                    builder,
                    scope,
                    refs,
                    failure,
                    depth + 1,
                )?);
            }
            // The helper has an independent native function body, not an
            // expanded/inlined syntax tree. A parameter may be read repeatedly
            // without re-evaluating any argument.
            let invocation = builder.ins().call(func_ref, &values);
            let results = builder.inst_results(invocation);
            if results.len() != 2 {
                return Err("E_LINK_CALL_ABI".into());
            }
            let result = results[0];
            let status = results[1];
            let bad = builder.ins().icmp_imm_s(IntCC::NotEqual, status, 0);
            let continuation = builder.create_block();
            builder
                .ins()
                .brif(bad, failure, &[BlockArg::Value(status)], continuation, &[]);
            builder.switch_to_block(continuation);
            builder.seal_block(continuation);
            Ok(result)
        }
    }
}

fn signature(module: &JITModule, helper_params: Option<usize>) -> cranelift_codegen::ir::Signature {
    let mut signature = module.make_signature();
    if let Some(n) = helper_params {
        for _ in 0..n {
            signature.params.push(AbiParam::new(types::I64));
        }
        signature.returns.push(AbiParam::new(types::I64));
        signature.returns.push(AbiParam::new(types::I32));
    } else {
        // Exact preexisting stage-0 native entry ABI.
        for _ in 0..4 {
            signature.params.push(AbiParam::new(types::I64));
        }
        signature
            .params
            .push(AbiParam::new(module.target_config().pointer_type()));
        signature.returns.push(AbiParam::new(types::I32));
    }
    signature
}

struct Prepared {
    module: JITModule,
    contexts: Vec<(FuncId, cranelift_codegen::Context)>,
    entry: FuncId,
    clif: String,
}

fn prepare(program: &[ScalarFn]) -> Result<Prepared, String> {
    validate(program)?;
    let jit = JITBuilder::new(default_libcall_names()).map_err(|e| e.to_string())?;
    let mut module = JITModule::new(jit);
    let mut ids = HashMap::new();
    let mut signatures = HashMap::new();
    for (index, function) in program.iter().enumerate() {
        let is_entry = function.name == "main";
        let sig = signature(
            &module,
            if is_entry {
                None
            } else {
                Some(function.params.len())
            },
        );
        let symbol = if is_entry {
            "atto_link_entry".to_owned()
        } else {
            format!("atto_link_helper_{index}")
        };
        let id = module
            .declare_function(&symbol, Linkage::Local, &sig)
            .map_err(|e| format!("E_LINK_DECLARE {e}"))?;
        ids.insert(function.name.clone(), id);
        signatures.insert(function.name.clone(), sig);
    }
    let entry = *ids.get("main").ok_or("E_LINK_MAIN_REQUIRED")?;
    let mut contexts = Vec::with_capacity(program.len());
    let mut clif = String::new();

    for function in program {
        let is_entry = function.name == "main";
        let id = ids[&function.name];
        let mut ctx = module.make_context();
        ctx.func.signature = signatures[&function.name].clone();
        // Imports are declared before any body is emitted, supporting
        // call-graph forward references while disallowing recursive cycles.
        let mut refs = HashMap::new();
        for target in program.iter().filter(|f| f.name != "main") {
            let func_id = ids[&target.name];
            let func_ref = module.declare_func_in_func(func_id, &mut ctx.func);
            refs.insert(target.name.clone(), func_ref);
        }

        let mut fbctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
        let start = b.create_block();
        let failed = b.create_block();
        b.append_block_params_for_function_params(start);
        b.append_block_param(failed, types::I32);
        b.switch_to_block(start);
        b.seal_block(start);
        let params = b.block_params(start).to_vec();
        let mut scope = HashMap::new();
        for (position, name) in function.params.iter().enumerate() {
            scope.insert(name.clone(), params[position]);
        }
        // Native parameters and val initializers are SSA values; every
        // initializer executes exactly once in the original lexical order.
        for (name, initializer) in &function.locals {
            let value = emit_expr(initializer, &mut b, &scope, &refs, failed, 0)?;
            scope.insert(name.clone(), value);
        }
        let value = emit_expr(&function.result, &mut b, &scope, &refs, failed, 0)?;
        let success = b.ins().iconst(types::I32, 0);
        if is_entry {
            b.ins().store(MemFlagsData::new(), value, params[4], 0);
            b.ins().return_(&[success]);
        } else {
            b.ins().return_(&[value, success]);
        }

        b.switch_to_block(failed);
        let status = b.block_params(failed)[0];
        if is_entry {
            b.ins().return_(&[status]);
        } else {
            let placeholder = b.ins().iconst(types::I64, 0);
            b.ins().return_(&[placeholder, status]);
        }
        b.seal_all_blocks();
        b.finalize(module.target_config());
        cranelift_codegen::verify_function(&ctx.func, module.isa())
            .map_err(|e| format!("E_LINK_CLIF_VERIFY {e}"))?;
        clif.push_str(&format!("; linked function {}\n", function.name));
        clif.push_str(&ctx.func.display().to_string());
        clif.push('\n');
        contexts.push((id, ctx));
    }
    Ok(Prepared {
        module,
        contexts,
        entry,
        clif,
    })
}

/// IR-only inspection: never define or finalize executable JIT allocations.
pub fn render_ir(program: &[ScalarFn]) -> Result<String, String> {
    Ok(prepare(program)?.clif)
}

pub fn compile(program: &[ScalarFn]) -> Result<Compiled, String> {
    let Prepared {
        mut module,
        contexts,
        entry,
        clif: _clif,
    } = prepare(program)?;
    for (id, mut context) in contexts {
        module
            .define_function(id, &mut context)
            .map_err(|e| format!("E_LINK_NATIVE_DEFINE {e}"))?;
        module.clear_context(&mut context);
    }
    module
        .finalize_definitions()
        .map_err(|e| format!("E_LINK_NATIVE_FINALIZE {e}"))?;
    Ok(Compiled {
        module,
        id: entry,
        arity: 0,
        #[cfg(test)]
        clif: _clif,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functional_pipelines::parse_linked_native;

    fn two_unit(source: &str, helper: &str) -> Vec<ScalarFn> {
        parse_linked_native(source, &[("twice".into(), "twice".into(), helper.into())])
            .expect("parse declared helper graph")
    }

    #[test]
    fn independent_native_helper_reuses_parameter_without_repeating_argument() {
        let funcs = two_unit(
            "pub fnc main(): int { return twice(21); }",
            "pub fnc twice(int x): int { return x + x; }",
        );
        validate(&funcs).expect("nonlinear parameter use now legal");
        assert_eq!(compile(&funcs).unwrap().run(&[]), Ok(42));
        let ir = render_ir(&funcs).unwrap();
        assert!(ir.contains("call "), "expected real CLIF call instruction");
    }

    #[test]
    fn reject_recursive_call_graph_even_if_dormant() {
        let funcs = two_unit(
            "pub fnc main(): int { return 42; }",
            "pub fnc twice(int x): int { return twice(x); }",
        );
        assert_eq!(validate(&funcs), Err("E_LINK_CALL_CYCLE".into()));
    }
}
