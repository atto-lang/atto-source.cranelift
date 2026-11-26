//! Typed, bounded leaf-function library profile. No imports, calls, mutable
//! locals, dynamic arithmetic, package linking, or synthetic main. Pure
//! comparisons and logical conjunction operate on proved int/bool values.
use super::{Parser, Token, integer_digits};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Value, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Int,
    Bool,
}
impl Ty {
    fn ir(self) -> cranelift_codegen::ir::Type {
        match self {
            Self::Int => types::I64,
            Self::Bool => types::I8,
        }
    }
}
enum ValueExpr {
    Int(i64),
    Bool(bool),
    Param(usize, Ty),
    Compare(IntCC, Box<Self>, Box<Self>),
    And(Box<Self>, Box<Self>),
}
impl ValueExpr {
    fn ty(&self) -> Ty {
        match self {
            Self::Int(_) => Ty::Int,
            Self::Param(_, ty) => *ty,
            Self::Bool(_) | Self::Compare(..) | Self::And(..) => Ty::Bool,
        }
    }
}
struct Leaf {
    name: String,
    params: Vec<(String, Ty)>,
    ret: Ty,
    body: ValueExpr,
}
pub struct Library {
    functions: Vec<Leaf>,
}

fn ty(p: &mut Parser<'_>) -> Result<Ty, String> {
    if p.take(Token::Word("int"))? {
        Ok(Ty::Int)
    } else if p.take(Token::Word("bool"))? {
        Ok(Ty::Bool)
    } else {
        Err("E_LIBRARY_TYPE: expected int or bool".into())
    }
}

fn atom(p: &mut Parser<'_>, params: &[(String, Ty)], depth: usize) -> Result<ValueExpr, String> {
    if depth > 32 {
        return Err("E_LIBRARY_DEPTH".into());
    }
    p.nodes += 1;
    if p.nodes > 128 {
        return Err("E_LIBRARY_EXPRESSION_BUDGET".into());
    }
    if p.take(Token::Punct(b'('))? {
        let value = expression(p, params, depth + 1)?;
        p.expect(Token::Punct(b')'))?;
        return Ok(value);
    }
    let neg = p.take(Token::Punct(b'-'))?;
    if let Token::Number(raw) = p.current {
        let digits = integer_digits(raw)?;
        let spelling = if neg { format!("-{digits}") } else { digits };
        let value = spelling.parse::<i64>().map_err(|_| "E_LIBRARY_INT_RANGE")?;
        p.next()?;
        return Ok(ValueExpr::Int(value));
    }
    if neg {
        return Err("E_LIBRARY_DYNAMIC_ARITHMETIC_UNSUPPORTED".into());
    }
    if p.take(Token::Word("true"))? {
        return Ok(ValueExpr::Bool(true));
    }
    if p.take(Token::Word("false"))? {
        return Ok(ValueExpr::Bool(false));
    }
    let name = p.name()?;
    let (index, (_, kind)) = params
        .iter()
        .enumerate()
        .find(|(_, (n, _))| *n == name)
        .ok_or("E_LIBRARY_UNBOUND")?;
    Ok(ValueExpr::Param(index, *kind))
}

fn integer(p: &mut Parser<'_>, params: &[(String, Ty)], depth: usize) -> Result<ValueExpr, String> {
    let mut lhs = atom(p, params, depth)?;
    let mut count = 0;
    while matches!(p.current, Token::Punct(b'+' | b'-')) {
        count += 1;
        if count > 64 {
            return Err("E_LIBRARY_EXPRESSION_BUDGET".into());
        }
        let plus = p.current == Token::Punct(b'+');
        p.next()?;
        let rhs = atom(p, params, depth)?;
        // Checked compile-time folding is legal only for two literal ints.
        // No runtime expression or caller argument is evaluated by the host.
        let (ValueExpr::Int(a), ValueExpr::Int(b)) = (lhs, rhs) else {
            return Err("E_LIBRARY_DYNAMIC_ARITHMETIC_UNSUPPORTED".into());
        };
        let v = if plus {
            a.checked_add(b)
        } else {
            a.checked_sub(b)
        };
        lhs = ValueExpr::Int(v.ok_or("E_LIBRARY_INT_RANGE")?);
    }
    Ok(lhs)
}

fn comparison(
    p: &mut Parser<'_>,
    params: &[(String, Ty)],
    depth: usize,
) -> Result<ValueExpr, String> {
    let lhs = integer(p, params, depth)?;
    let op = match p.current {
        Token::Punct(b'<') => Some(false),
        Token::Punct(b'>') => Some(true),
        _ => None,
    };
    let Some(greater) = op else {
        return Ok(lhs);
    };
    p.next()?;
    let equal = p.take(Token::Punct(b'='))?;
    let rhs = integer(p, params, depth)?;
    if lhs.ty() != Ty::Int || rhs.ty() != Ty::Int {
        return Err("E_LIBRARY_COMPARE_TYPE".into());
    }
    let cc = match (greater, equal) {
        (true, true) => IntCC::SignedGreaterThanOrEqual,
        (true, false) => IntCC::SignedGreaterThan,
        (false, true) => IntCC::SignedLessThanOrEqual,
        (false, false) => IntCC::SignedLessThan,
    };
    Ok(ValueExpr::Compare(cc, Box::new(lhs), Box::new(rhs)))
}

fn expression(
    p: &mut Parser<'_>,
    params: &[(String, Ty)],
    depth: usize,
) -> Result<ValueExpr, String> {
    let mut lhs = comparison(p, params, depth)?;
    let mut count = 0;
    while p.take(Token::Punct(b'&'))? {
        count += 1;
        if count > 64 {
            return Err("E_LIBRARY_EXPRESSION_BUDGET".into());
        }
        p.expect(Token::Punct(b'&'))?;
        let rhs = comparison(p, params, depth)?;
        if lhs.ty() != Ty::Bool || rhs.ty() != Ty::Bool {
            return Err("E_LIBRARY_BOOL_TYPE".into());
        }
        lhs = ValueExpr::And(Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

/// Only select a library profile from a lexed namespace and non-main public
/// function header. Once selected, malformed or unsupported source cannot fall
/// back to another parser. Keep the existing main/import and strict-AOT lanes.
pub fn parse_if_library(source: &str) -> Result<Option<Library>, String> {
    let mut p = Parser::new(source)?;
    if !p.take(Token::Word("use"))? || !p.take(Token::Word("namespace"))? {
        return Ok(None);
    }
    let _namespace = p.name()?;
    p.expect(Token::Punct(b';'))?;
    if !p.take(Token::Word("pub"))? || !p.take(Token::Word("fnc"))? {
        return Ok(None);
    }
    if p.current == Token::Word("main") {
        return Ok(None);
    }
    let mut names = HashSet::new();
    let mut functions = Vec::new();
    loop {
        if functions.len() >= 32 {
            return Err("E_LIBRARY_FUNCTION_BUDGET".into());
        }
        let name = p.name()?;
        if name == "main" || !names.insert(name.clone()) {
            return Err("E_LIBRARY_DUP_OR_ENTRY".into());
        }
        p.expect(Token::Punct(b'('))?;
        let mut params = Vec::new();
        let mut param_names = HashSet::new();
        if !p.take(Token::Punct(b')'))? {
            loop {
                if params.len() >= 4 {
                    return Err("E_LIBRARY_ARITY".into());
                }
                let (name, kind) = if matches!(p.current, Token::Word("int" | "bool")) {
                    let kind = ty(&mut p)?;
                    (p.name()?, kind)
                } else {
                    let name = p.name()?;
                    p.expect(Token::Punct(b':'))?;
                    (name, ty(&mut p)?)
                };
                // The bounded library ABI currently only accepts int parameters;
                // this avoids noncanonical booleans crossing the native ABI.
                if kind != Ty::Int || !param_names.insert(name.clone()) {
                    return Err("E_LIBRARY_PARAMETER_TYPE_OR_DUP".into());
                }
                params.push((name, kind));
                if p.take(Token::Punct(b')'))? {
                    break;
                }
                p.expect(Token::Punct(b','))?;
            }
        }
        if p.take(Token::Punct(b'-'))? {
            p.expect(Token::Punct(b'>'))?;
        } else {
            p.expect(Token::Punct(b':'))?;
        }
        let ret = ty(&mut p)?;
        p.expect(Token::Punct(b'{'))?;
        p.expect(Token::Word("return"))?;
        p.nodes = 0;
        let body = expression(&mut p, &params, 0)?;
        if body.ty() != ret {
            return Err("E_LIBRARY_RETURN_TYPE".into());
        }
        p.expect(Token::Punct(b';'))?;
        p.expect(Token::Punct(b'}'))?;
        functions.push(Leaf {
            name,
            params,
            ret,
            body,
        });
        if p.current == Token::End {
            break;
        }
        p.expect(Token::Word("pub"))?;
        p.expect(Token::Word("fnc"))?;
    }
    Ok(Some(Library { functions }))
}

fn emit(expr: &ValueExpr, b: &mut FunctionBuilder<'_>, params: &[Value]) -> Value {
    match expr {
        ValueExpr::Int(n) => b.ins().iconst(types::I64, *n),
        ValueExpr::Bool(v) => b.ins().iconst(types::I8, i64::from(*v)),
        ValueExpr::Param(i, _) => params[*i],
        ValueExpr::Compare(cc, a, c) => {
            let a = emit(a, b, params);
            let c = emit(c, b, params);
            b.ins().icmp(*cc, a, c)
        }
        ValueExpr::And(a, c) => {
            let a = emit(a, b, params);
            let c = emit(c, b, params);
            // Both operands are pure, total comparisons/booleans, with no
            // runtime arithmetic traps or calls. Eager band is equivalent.
            b.ins().band(a, c)
        }
    }
}

impl Library {
    pub fn compile(&self, object: bool) -> Result<(String, Vec<u8>), String> {
        let mut flags = settings::builder();
        flags.set("is_pic", "true").map_err(|e| e.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|e| e.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|e| e.to_string())?;
        let builder = ObjectBuilder::new(isa, "atto_scalar_library", default_libcall_names())
            .map_err(|e| e.to_string())?;
        let mut module = ObjectModule::new(builder);
        let mut ir = String::new();
        for f in &self.functions {
            let mut ctx = module.make_context();
            for (_, ty) in &f.params {
                ctx.func.signature.params.push(AbiParam::new(ty.ir()));
            }
            ctx.func.signature.returns.push(AbiParam::new(f.ret.ir()));
            let id = module
                .declare_function(&f.name, Linkage::Export, &ctx.func.signature)
                .map_err(|e| e.to_string())?;
            let mut fb = FunctionBuilderContext::new();
            {
                let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb);
                let entry = b.create_block();
                b.append_block_params_for_function_params(entry);
                b.switch_to_block(entry);
                b.seal_block(entry);
                let params = b.block_params(entry).to_vec();
                let result = emit(&f.body, &mut b, &params);
                b.ins().return_(&[result]);
                b.finalize(module.target_config());
            }
            cranelift_codegen::verify_function(&ctx.func, module.isa())
                .map_err(|e| e.to_string())?;
            ir.push_str(&format!("; {}\n{}\n", f.name, ctx.func.display()));
            if object {
                module
                    .define_function(id, &mut ctx)
                    .map_err(|e| e.to_string())?;
            }
        }
        let bytes = if object {
            module.finish().emit().map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        Ok((ir, bytes))
    }
}
