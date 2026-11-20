//! Strict pure-int64 Atto functional-pipeline expansion into the existing
//! native Cranelift Expr IR. No host expression evaluation or JIT fallback.
 //! Only single-use lexical bindings/call parameters are admitted, because
//! the stage-0 Expr IR has no let node; this keeps checked evaluation order
//! and exact-once semantics for the admitted subset.
use crate::frontend::{Expr, Program};
use std::collections::HashMap;

const MAX_SOURCE: usize = 16 * 1024;
const MAX_TOKENS: usize = 512;
const MAX_DEPTH: usize = 32;
const MAX_EXPANSION: usize = 2048;

#[derive(Clone)]
enum Raw {
    Literal(i64),
    Name(String),
    Add(Box<Raw>, Box<Raw>),
    Sub(Box<Raw>, Box<Raw>),
    Mul(Box<Raw>, Box<Raw>),
    Call(String, Vec<Raw>),
}
struct Fn {
    name: String,
    params: Vec<String>,
    locals: Vec<(String, Raw)>,
    result: Raw,
}
fn lex(text: &str) -> Result<Vec<String>, String> {
    if text.len() > MAX_SOURCE { return Err("E_PIPE_SOURCE_LIMIT".into()); }
    if !text.is_ascii() { return Err("E_PIPE_ASCII_SUBSET".into()); }
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => { i += 1; continue; }
            b'/' if bytes.get(i+1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
                continue;
            }
            b'/' if bytes.get(i+1) == Some(&b'*') => {
                i += 2;
                let mut found = false;
                while i+1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i+1] == b'/' {
                        found = true; i += 2; break;
                    }
                    i += 1;
                }
                if !found { return Err("E_PIPE_LEX_COMMENT".into()); }
                continue;
            }
            _ => {}
        }
        if tokens.len() >= MAX_TOKENS { return Err("E_PIPE_TOKEN_LIMIT".into()); }
        let start = i;
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
        } else if bytes[i].is_ascii_digit() {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
        } else if bytes[i..].starts_with(b"|>")
            || bytes[i..].starts_with(b"<|")
            || bytes[i..].starts_with(b"->") {
            i += 2;
        } else if b"(){}:,;+-*=".contains(&bytes[i]) {
            i += 1;
        } else { return Err("E_PIPE_LEX_UNSUPPORTED".into()); }
        tokens.push(text[start..i].to_owned());
    }
    tokens.push("<eof>".into());
    Ok(tokens)
}
struct Parser { tokens: Vec<String>, at: usize }
impl Parser {
    fn new(src: &str) -> Result<Self, String> {
        Ok(Self { tokens: lex(src)?, at: 0 })
    }
    fn peek(&self) -> &str { &self.tokens[self.at] }
    fn eat(&mut self, expected: &str) -> bool {
        if self.peek() != expected { return false; }
        self.at += 1;
        true
    }
    fn need(&mut self, expected: &str) -> Result<(), String> {
        if !self.eat(expected) { return Err(format!("E_PIPE_PARSE expected {expected}")); }
        Ok(())
    }
    fn name(&mut self) -> Result<String, String> {
        let word = self.peek();
        if !word.bytes().next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            || !word.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || matches!(word, "pub" | "fnc" | "int" | "val" | "let"
                      | "return" | "true" | "false" | "bool" | "actor"
                      | "unsafe" | "await" | "susp" | "suspends" | "async"
                      | "critical" | "trap" | "panic" | "raise" | "throw"
                      | "take" | "borrow" | "copy" | "share" | "drop"
                      | "pin" | "rt" | "io" | "stdio" | "process"
                      | "spawn" | "select" | "import" | "module" | "define"
                      | "namespace" | "use" | "pipe" | "rpipe") {
            return Err("E_PIPE_NAME".into());
        }
        let name = word.to_owned();
        self.at += 1;
        Ok(name)
    }
    fn args(&mut self, depth: usize) -> Result<Vec<Raw>, String> {
        self.need("(")?;
        let mut args = Vec::new();
        if self.peek() != ")" {
            loop {
                args.push(self.expr(0, depth + 1, None)?);
                if args.len() > 4 { return Err("E_PIPE_ARITY_LIMIT".into()); }
                if !self.eat(",") { break; }
            }
        }
        self.need(")")?;
        Ok(args)
    }
    fn expr(&mut self, min: u8, depth: usize, direction: Option<&str>) -> Result<Raw, String> {
        if depth > MAX_DEPTH { return Err("E_PIPE_DEPTH".into()); }
        let mut lhs = if self.eat("(") {
            let inner = self.expr(0, depth + 1, None)?;
            self.need(")")?;
            inner
        } else if self.eat("-") {
            // The signed i64 minimum is admitted as a negative literal.
            // Positive 2^63 must still fail, so it cannot be parsed first
            // as an i64 and then negated.
            if self.peek() == "9223372036854775808" {
                self.at += 1;
                Raw::Literal(i64::MIN)
            } else {
                Raw::Sub(Box::new(Raw::Literal(0)),
                         Box::new(self.expr(30, depth + 1, direction)?))
            }
        } else if self.peek().as_bytes().first().is_some_and(u8::is_ascii_digit) {
            let n = self.peek().parse::<i64>().map_err(|_| "E_PIPE_LITERAL")?;
            self.at += 1;
            Raw::Literal(n)
        } else if self.peek() == "true" || self.peek() == "false" {
            return Err("E_PIPE_TYPE: Boolean cannot inhabit int64".into());
        } else {
            let name = self.name()?;
            if self.peek() == "(" { Raw::Call(name, self.args(depth)?) }
            else { Raw::Name(name) }
        };
        let mut pipe = direction.map(str::to_owned);
        loop {
            let op = self.peek().to_owned();
            let bp = match op.as_str() {
                "*" => 20, "+" | "-" => 10, "|>" | "<|" => 5, _ => 0
            };
            if bp == 0 || bp < min { break; }
            self.at += 1;
            lhs = match op.as_str() {
                "+" => Raw::Add(Box::new(lhs), Box::new(self.expr(bp+1, depth+1, pipe.as_deref())?)),
                "-" => Raw::Sub(Box::new(lhs), Box::new(self.expr(bp+1, depth+1, pipe.as_deref())?)),
                "*" => Raw::Mul(Box::new(lhs), Box::new(self.expr(bp+1, depth+1, pipe.as_deref())?)),
                "|>" | "<|" => {
                    if pipe.as_deref().is_some_and(|existing| existing != op) {
                        return Err("E_PIPE_MIXED_DIRECTION".into());
                    }
                    pipe = Some(op.clone());
                    if op == "|>" {
                        let target = self.name()?;
                        let mut args = if self.peek() == "(" { self.args(depth+1)? } else { Vec::new() };
                        if args.len() >= 4 { return Err("E_PIPE_ARITY_LIMIT".into()); }
                        args.insert(0, lhs);
                        Raw::Call(target, args)
                    } else {
                        let (target, mut args) = match lhs {
                            Raw::Name(name) => (name, Vec::new()),
                            Raw::Call(name, args) => (name, args),
                            _ => return Err("E_PIPE_CALLEE".into()),
                        };
                        if args.len() >= 4 { return Err("E_PIPE_ARITY_LIMIT".into()); }
                        let value = self.expr(bp, depth+1, pipe.as_deref())?;
                        args.insert(0, value);
                        Raw::Call(target, args)
                    }
                }
                _ => unreachable!(),
            };
        }
        Ok(lhs)
    }
    fn body(&mut self, locals: &mut Vec<(String, Raw)>) -> Result<Raw, String> {
        while self.eat("val") {
            let n = self.name()?;
            self.need("=")?;
            let value = self.expr(0, 0, None)?;
            self.need(";")?;
            if locals.len() >= 4 { return Err("E_PIPE_LOCAL_LIMIT".into()); }
            locals.push((n, value));
        }
        self.need("return")?;
        let result = self.expr(0, 0, None)?;
        self.need(";")?;
        self.need("}")?;
        Ok(result)
    }
    fn parse(mut self) -> Result<Vec<Fn>, String> {
        let mut fns = Vec::new();
        while self.peek() != "<eof>" {
            self.eat("pub");
            self.need("fnc")?;
            let name = self.name()?;
            self.need("(")?;
            let mut params = Vec::new();
            if self.peek() != ")" {
                loop {
                    self.need("int")?;
                    let n = self.name()?;
                    if params.contains(&n) { return Err("E_PIPE_DUP_PARAM".into()); }
                    params.push(n);
                    if params.len() > 4 { return Err("E_PIPE_ARITY_LIMIT".into()); }
                    if !self.eat(",") { break; }
                }
            }
            self.need(")")?;
            let colon = self.eat(":");
            if !colon { self.need("->")?; }
            self.need("int")?;
            let mut locals = Vec::new();
            let result = if self.eat("{") {
                self.body(&mut locals)?
            } else if colon && self.eat("->") {
                if self.eat("{") { self.body(&mut locals)? }
                else {
                    let x = self.expr(0, 0, None)?;
                    self.need(";")?;
                    x
                }
            } else { return Err("E_PIPE_BODY".into()); };
            fns.push(Fn { name, params, locals, result });
            if fns.len() > 16 { return Err("E_PIPE_FUNCTION_LIMIT".into()); }
        }
        Ok(fns)
    }
}
struct Lower<'a> {
    functions: &'a HashMap<String, Fn>,
    budget: usize,
    stack: Vec<String>,
}
impl Lower<'_> {
    fn lowered(&mut self, raw: &Raw, env: &HashMap<String, Expr>,
               used: &mut HashMap<String, usize>,
               order: &mut Vec<String>, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH || self.budget == 0 { return Err("E_PIPE_EXPANSION_LIMIT".into()); }
        self.budget -= 1;
        Ok(match raw {
            Raw::Literal(v) => Expr::Literal(*v),
            Raw::Name(n) => {
                let value = env.get(n).ok_or("E_PIPE_SYMBOL")?;
                *used.entry(n.clone()).or_default() += 1;
                order.push(n.clone());
                value.clone()
            }
            Raw::Add(a,b) => Expr::Add(
                Box::new(self.lowered(a,env,used,order,depth+1)?),
                Box::new(self.lowered(b,env,used,order,depth+1)?)),
            Raw::Sub(a,b) => Expr::Sub(
                Box::new(self.lowered(a,env,used,order,depth+1)?),
                Box::new(self.lowered(b,env,used,order,depth+1)?)),
            Raw::Mul(a,b) => Expr::Mul(
                Box::new(self.lowered(a,env,used,order,depth+1)?),
                Box::new(self.lowered(b,env,used,order,depth+1)?)),
            Raw::Call(name,args) => {
                let mut params = Vec::new();
                for arg in args { params.push(self.lowered(arg,env,used,order,depth+1)?); }
                self.call(name, params, depth+1)?
            }
        })
    }
    fn call(&mut self, name: &str, args: Vec<Expr>, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH { return Err("E_PIPE_EXPANSION_LIMIT".into()); }
        let func = self.functions.get(name).ok_or("E_PIPE_SYMBOL")?;
        if func.params.len() != args.len() { return Err("E_PIPE_ARITY".into()); }
        if self.stack.iter().any(|s| s == name) { return Err("E_PIPE_CYCLE".into()); }
        self.stack.push(name.to_owned());
        let result = self.call_body(func, args, depth);
        self.stack.pop();
        result
    }
    fn call_body(&mut self, func: &Fn, args: Vec<Expr>, depth: usize) -> Result<Expr, String> {
        let mut env: HashMap<String, Expr> = func.params.iter().cloned().zip(args).collect();
        let mut used: HashMap<String, usize> = HashMap::new();
        let mut order = Vec::new();
        for (n, expr) in &func.locals {
            if env.contains_key(n) { return Err("E_PIPE_DUP_LOCAL".into()); }
            let value = self.lowered(expr,&env,&mut used,&mut order,depth+1)?;
            env.insert(n.clone(), value);
        }
        let answer = self.lowered(&func.result,&env,&mut used,&mut order,depth+1)?;
        // Without a Cranelift let node or explicit function call IR, only
        // linearly consumed bindings are safe to substitute as pure trees.
        for n in func.params.iter().chain(func.locals.iter().map(|(n,_)| n)) {
            if used.get(n).copied() != Some(1) {
                return Err("E_PIPE_NONLINEAR_BINDING_UNSUPPORTED".into());
            }
        }
        // The call ABI evaluates argument expressions left-to-right before
        // entering the callee. Inlined parameter expressions must be observed
        // in their declaration order, or early checked failures could reorder.
        let parameter_uses: Vec<&str> = order.iter()
            .filter(|n| func.params.iter().any(|p| p == *n))
            .map(String::as_str).collect();
        let declared_params: Vec<&str> = func.params.iter().map(String::as_str).collect();
        if parameter_uses != declared_params {
            return Err("E_PIPE_ARGUMENT_ORDER_UNSUPPORTED".into());
        }
        // Every eager val initializer must execute before later val bodies
        // and before the return expression. Enforce order of uses.
        let observed: Vec<&str> = order.iter()
            .filter(|n| func.locals.iter().any(|(declared, _)| declared == *n))
            .map(String::as_str).collect();
        let declared: Vec<&str> = func.locals.iter().map(|(n,_)| n.as_str()).collect();
        if observed != declared { return Err("E_PIPE_LOCAL_ORDER_UNSUPPORTED".into()); }
        Ok(answer)
    }
}
/// Compile a strict pure-i64 subset of named Atto functions by **checked**
/// static inlining, then let the existing Cranelift backend generate actual
/// machine code. This path is NOT an interpreter, general linker, or AOT.
pub fn parse(source: &str) -> Result<Program, String> {
    let functions = Parser::new(source)?.parse()?;
    let mut names = HashMap::new();
    for f in functions {
        if names.insert(f.name.clone(), f).is_some() { return Err("E_PIPE_DUP_FUNCTION".into()); }
    }
    let main = names.get("main").ok_or("E_PIPE_ENTRY")?;
    if !main.params.is_empty() { return Err("E_PIPE_ENTRY".into()); }
    // Also check dormant function bodies; reject errors before CLIF output.
    let mut sorted_names: Vec<_> = names.keys().cloned().collect();
    sorted_names.sort();
    for name in &sorted_names {
        let f = &names[name];
        let args = f.params.iter().map(|_| Expr::Literal(0)).collect();
        let mut lower = Lower{functions:&names,budget:MAX_EXPANSION,stack:Vec::new()};
        lower.call(name, args, 0)?;
    }
    let mut lower = Lower{functions:&names,budget:MAX_EXPANSION,stack:Vec::new()};
    let result = lower.call("main",Vec::new(),0)?;
    Ok(Program { parameters: Vec::new(), result })
}
