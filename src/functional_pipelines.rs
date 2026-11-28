//! Strict pure-int64 Atto functional-pipeline expansion into the existing
//! native Cranelift Expr IR. No host expression evaluation or JIT fallback.
//! Only single-use lexical bindings/call parameters are admitted, because
//! the stage-0 Expr IR has no let node; this keeps checked evaluation order
//! and exact-once semantics for the admitted subset.
use crate::frontend::{
    Expr, Program, skip_block_comment, validate_ast_depth, validate_source_envelope,
};
use std::collections::HashMap;

const MAX_SOURCE: usize = 16 * 1024;
const MAX_TOKENS: usize = 512;
const MAX_DEPTH: usize = 32;
const MAX_EXPANSION: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarType {
    Int,
    Bool,
}

#[derive(Clone)]
pub(crate) enum Raw {
    Literal(i64),
    Bool(bool),
    Not(Box<Raw>),
    Eq(Box<Raw>, Box<Raw>),
    Ne(Box<Raw>, Box<Raw>),
    BitAnd(Box<Raw>, Box<Raw>),
    Lt(Box<Raw>, Box<Raw>),
    Le(Box<Raw>, Box<Raw>),
    Gt(Box<Raw>, Box<Raw>),
    Ge(Box<Raw>, Box<Raw>),
    And(Box<Raw>, Box<Raw>),
    Or(Box<Raw>, Box<Raw>),
    Xor(Box<Raw>, Box<Raw>),
    If(Box<Raw>, Box<Raw>, Box<Raw>),
    Name(String),
    Add(Box<Raw>, Box<Raw>),
    Sub(Box<Raw>, Box<Raw>),
    Mul(Box<Raw>, Box<Raw>),
    Call(String, Vec<Raw>),
    // Call-operand metadata; never admitted as an ordinary expression.
    NamedActual(String, Box<Raw>),
    Tuple(Vec<Raw>),
    TuplePattern(Vec<Raw>, Vec<Option<String>>, Box<Raw>),
}
impl Raw {
    fn requires_tuple_lowering(&self) -> bool {
        match self {
            Self::Tuple(_) | Self::TuplePattern(_, _, _) | Self::NamedActual(_, _) => true,
            Self::Literal(_) | Self::Bool(_) | Self::Name(_) => false,
            Self::Not(v) => v.requires_tuple_lowering(),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Eq(a, b)
            | Self::Ne(a, b)
            | Self::BitAnd(a, b)
            | Self::Lt(a, b)
            | Self::Le(a, b)
            | Self::Gt(a, b)
            | Self::Ge(a, b)
            | Self::And(a, b)
            | Self::Or(a, b)
            | Self::Xor(a, b) => a.requires_tuple_lowering() || b.requires_tuple_lowering(),
            Self::If(c, y, n) => {
                c.requires_tuple_lowering()
                    || y.requires_tuple_lowering()
                    || n.requires_tuple_lowering()
            }
            Self::Call(_, args) => args.iter().any(Self::requires_tuple_lowering),
        }
    }
}
pub(crate) fn requires_tuple_lowering(functions: &[Fn]) -> bool {
    functions.iter().any(|f| {
        f.result.requires_tuple_lowering()
            || f.locals.iter().any(|(_, v)| v.requires_tuple_lowering())
    })
}
// Dispatch a fully parsed single-main source to the typed linked backend
// when it contains a named reduction. Never grant admission from raw text.
fn has_named_boolean_call(expr: &Raw) -> bool {
    match expr {
        Raw::Call(name, args) => {
            matches!(name.as_str(), "And" | "Or" | "Xor") || args.iter().any(has_named_boolean_call)
        }
        Raw::Not(v) | Raw::NamedActual(_, v) => has_named_boolean_call(v),
        Raw::Add(a, b)
        | Raw::Sub(a, b)
        | Raw::Mul(a, b)
        | Raw::Eq(a, b)
        | Raw::Ne(a, b)
        | Raw::BitAnd(a, b)
        | Raw::Lt(a, b)
        | Raw::Le(a, b)
        | Raw::Gt(a, b)
        | Raw::Ge(a, b)
        | Raw::And(a, b)
        | Raw::Or(a, b)
        | Raw::Xor(a, b) => has_named_boolean_call(a) || has_named_boolean_call(b),
        Raw::If(c, y, n) => {
            has_named_boolean_call(c) || has_named_boolean_call(y) || has_named_boolean_call(n)
        }
        Raw::Tuple(values) => values.iter().any(has_named_boolean_call),
        Raw::TuplePattern(values, _, body) => {
            values.iter().any(has_named_boolean_call) || has_named_boolean_call(body)
        }
        Raw::Literal(_) | Raw::Bool(_) | Raw::Name(_) => false,
    }
}

pub(crate) fn requires_named_boolean_lowering(functions: &[Fn]) -> bool {
    functions.iter().any(|f| {
        has_named_boolean_call(&f.result) || f.locals.iter().any(|(_, v)| has_named_boolean_call(v))
    })
}

pub(crate) struct Fn {
    pub(crate) public: bool,
    pub(crate) name: String,
    pub(crate) params: Vec<String>,
    pub(crate) param_types: Vec<ScalarType>,
    pub(crate) return_type: ScalarType,
    pub(crate) locals: Vec<(String, Raw)>,
    pub(crate) result: Raw,
}
// Keep the alternate scalar frontend aligned with the primary lexer.
// These are nominal sum types/constructors, never scalar callable aliases.
pub(crate) fn reserved_sum_name(word: &str) -> bool {
    matches!(word, "Option" | "Result" | "Some" | "None" | "Ok" | "Err")
}
fn lex(text: &str) -> Result<Vec<String>, String> {
    if text.len() > MAX_SOURCE {
        return Err("E_PIPE_SOURCE_LIMIT".into());
    }
    if !text.is_ascii() {
        return Err("E_PIPE_ASCII_SUBSET".into());
    }
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = skip_block_comment(bytes, i)?;
                continue;
            }
            _ => {}
        }
        if tokens.len() >= MAX_TOKENS {
            return Err("E_PIPE_TOKEN_LIMIT".into());
        }
        let start = i;
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else if bytes[i].is_ascii_digit() {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"|>")
            || bytes[i..].starts_with(b"<|")
            || bytes[i..].starts_with(b"->")
            || bytes[i..].starts_with(b"&&")
            || bytes[i..].starts_with(b"||")
            || bytes[i..].starts_with(b"==")
            || bytes[i..].starts_with(b"!=")
            || bytes[i..].starts_with(b"<=")
            || bytes[i..].starts_with(b">=")
        {
            i += 2;
        } else if b"(){}:,;+-*=!|^&<>".contains(&bytes[i]) {
            i += 1;
        } else {
            return Err("E_PIPE_LEX_UNSUPPORTED".into());
        }
        let token = &text[start..i];
        if reserved_sum_name(token) {
            return Err("E_PIPE_TAGGED_SUM_UNSUPPORTED: typed Option/Result/trap is unsupported by the Cranelift scalar backend".into());
        }
        tokens.push(token.to_owned());
    }
    tokens.push("<eof>".into());
    Ok(tokens)
}
pub(crate) const LINKED_MAX_ARITY: usize = 8;
struct Parser {
    tokens: Vec<String>,
    at: usize,
    max_arity: usize,
}
impl Parser {
    fn new(src: &str, max_arity: usize) -> Result<Self, String> {
        Ok(Self {
            tokens: lex(src)?,
            at: 0,
            max_arity,
        })
    }
    fn peek(&self) -> &str {
        &self.tokens[self.at]
    }
    fn eat(&mut self, expected: &str) -> bool {
        if self.peek() != expected {
            return false;
        }
        self.at += 1;
        true
    }
    fn need(&mut self, expected: &str) -> Result<(), String> {
        if !self.eat(expected) {
            return Err(format!("E_PIPE_PARSE expected {expected}"));
        }
        Ok(())
    }
    fn name(&mut self) -> Result<String, String> {
        let word = self.peek();
        if !word
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            || !word.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || matches!(
                word,
                "pub"
                    | "fnc"
                    | "int"
                    | "val"
                    | "let"
                    | "return"
                    | "true"
                    | "false"
                    | "bool"
                    | "boolean"
                    | "and"
                    | "or"
                    | "xor"
                    | "not"
                    | "eq"
                    | "neq"
                    | "if"
                    | "then"
                    | "do"
                    | "fi"
                    | "else"
                    | "actor"
                    | "unsafe"
                    | "await"
                    | "susp"
                    | "suspends"
                    | "async"
                    | "critical"
                    | "trap"
                    | "panic"
                    | "raise"
                    | "throw"
                    | "take"
                    | "borrow"
                    | "copy"
                    | "share"
                    | "drop"
                    | "pin"
                    | "rt"
                    | "io"
                    | "stdio"
                    | "process"
                    | "spawn"
                    | "select"
                    | "import"
                    | "module"
                    | "define"
                    | "namespace"
                    | "use"
                    | "pipe"
                    | "rpipe"
                    | "sync"
                    | "task"
                    | "yield"
                    | "freeze"
                    | "weak"
                    | "ref"
                    | "deref"
                    | "unref"
                    | "seq"
                    | "new"
                    | "macro"
                    | "const"
                    | "match"
                    | "Mutex"
                    | "Record"
                    | "Map"
                    | "Entry"
            )
        {
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
                let label = if self.tokens.get(self.at + 1).is_some_and(|s| s == ":" || s == "=") {
                    let name = self.name()?;
                    let colon = self.eat(":");
                    if !colon { self.need("=")?; }
                    if colon && self.peek() == "=" {
                        return Err("E_LINK_PAIR_UNSUPPORTED".into());
                    }
                    Some(name)
                } else { None };
                let value = self.expr(0, depth + 1, None)?;
                args.push(match label {
                    Some(name) => Raw::NamedActual(name, Box::new(value)),
                    None => value,
                });
                if args.len() > self.max_arity {
                    return Err("E_PIPE_ARITY_LIMIT".into());
                }
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.need(")")?;
        Ok(args)
    }
    fn expr(&mut self, min: u8, depth: usize, direction: Option<&str>) -> Result<Raw, String> {
        if depth > MAX_DEPTH {
            return Err("E_PIPE_DEPTH".into());
        }
        let mut lhs = if self.eat("(") {
            if self.eat(")") {
                Raw::Tuple(Vec::new())
            } else {
                let inner = self.expr(0, depth + 1, None)?;
                if self.eat(",") {
                    let mut items = vec![inner];
                    while self.peek() != ")" {
                        if items.len() >= 8 {
                            return Err("E_PIPE_TUPLE_LIMIT".into());
                        }
                        items.push(self.expr(0, depth + 1, None)?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.need(")")?;
                    Raw::Tuple(items)
                } else {
                    self.need(")")?;
                    inner
                }
            }
        } else if self.eat("!") || self.eat("not") {
            Raw::Not(Box::new(self.expr(30, depth + 1, direction)?))
        } else if self.eat("-") {
            // The signed i64 minimum is admitted as a negative literal.
            // Positive 2^63 must still fail, so it cannot be parsed first
            // as an i64 and then negated.
            if self.peek() == "9223372036854775808" {
                self.at += 1;
                Raw::Literal(i64::MIN)
            } else {
                Raw::Sub(
                    Box::new(Raw::Literal(0)),
                    Box::new(self.expr(30, depth + 1, direction)?),
                )
            }
        } else if self
            .peek()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
        {
            let n = self.peek().parse::<i64>().map_err(|_| "E_PIPE_LITERAL")?;
            self.at += 1;
            Raw::Literal(n)
        } else if self.eat("true") {
            Raw::Bool(true)
        } else if self.eat("false") {
            Raw::Bool(false)
        } else {
            let name = self.name()?;
            if self.peek() == "(" {
                Raw::Call(name, self.args(depth)?)
            } else {
                Raw::Name(name)
            }
        };
        let mut pipe = direction.map(str::to_owned);
        loop {
            let op = self.peek().to_owned();
            let bp = match op.as_str() {
                "*" => 20,
                "+" | "-" => 14,
                "<" | "<=" | ">" | ">=" => 13,
                "eq" | "neq" | "==" | "!=" => 12,
                "&" => 11,
                "and" | "&&" => 8,
                "xor" => 7,
                "or" | "||" => 6,
                "|>" | "<|" => 5,
                _ => 0,
            };
            if bp == 0 || bp < min {
                break;
            }
            self.at += 1;
            lhs = match op.as_str() {
                "+" => Raw::Add(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "-" => Raw::Sub(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "*" => Raw::Mul(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "&" => Raw::BitAnd(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "<" => Raw::Lt(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "<=" => Raw::Le(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                ">" => Raw::Gt(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                ">=" => Raw::Ge(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "eq" | "==" => Raw::Eq(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "neq" | "!=" => Raw::Ne(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "and" | "&&" => Raw::And(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "or" | "||" => Raw::Or(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "xor" => Raw::Xor(
                    Box::new(lhs),
                    Box::new(self.expr(bp + 1, depth + 1, pipe.as_deref())?),
                ),
                "|>" | "<|" => {
                    if pipe.as_deref().is_some_and(|existing| existing != op) {
                        return Err("E_PIPE_MIXED_DIRECTION".into());
                    }
                    pipe = Some(op.clone());
                    if op == "|>" {
                        if self.eat("|") {
                            let values = match lhs {
                                Raw::Tuple(values) => values,
                                _ => return Err("E_PIPE_PATTERN_SHAPE".into()),
                            };
                            self.need("(")?;
                            let mut bindings = Vec::new();
                            let mut names = std::collections::HashSet::new();
                            if !self.eat(")") {
                                loop {
                                    let n = self.name()?;
                                    if bindings.len() >= 8 {
                                        return Err("E_PIPE_TUPLE_LIMIT".into());
                                    }
                                    if n == "self"
                                        || n == "arguments"
                                        || (n != "_" && !names.insert(n.clone()))
                                    {
                                        return Err("E_PIPE_DUP_PARAM".into());
                                    }
                                    if self.eat(":") {
                                        if n == "_" {
                                            return Err("E_PIPE_PATTERN".into());
                                        }
                                        self.need("int")?;
                                    }
                                    bindings.push(if n == "_" { None } else { Some(n) });
                                    if !self.eat(",") {
                                        if bindings.len() == 1 {
                                            return Err("E_PIPE_PATTERN".into());
                                        }
                                        self.need(")")?;
                                        break;
                                    }
                                    if self.eat(")") {
                                        break;
                                    }
                                }
                            }
                            if values.len() != bindings.len() {
                                return Err("E_PIPE_ARITY".into());
                            }
                            self.need("|")?;
                            self.need("->")?;
                            let body = self.expr(bp + 1, depth + 1, None)?;
                            lhs = Raw::TuplePattern(values, bindings, Box::new(body));
                            continue;
                        }
                        let target = self.name()?;
                        let mut args = if self.peek() == "(" {
                            self.args(depth + 1)?
                        } else {
                            Vec::new()
                        };
                        if args.len() >= 4 {
                            return Err("E_PIPE_ARITY_LIMIT".into());
                        }
                        args.insert(0, lhs);
                        Raw::Call(target, args)
                    } else {
                        let (target, mut args) = match lhs {
                            Raw::Name(name) => (name, Vec::new()),
                            Raw::Call(name, args) => (name, args),
                            _ => return Err("E_PIPE_CALLEE".into()),
                        };
                        if args.len() >= 4 {
                            return Err("E_PIPE_ARITY_LIMIT".into());
                        }
                        let value = self.expr(bp, depth + 1, pipe.as_deref())?;
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
            if locals.len() >= 4 {
                return Err("E_PIPE_LOCAL_LIMIT".into());
            }
            locals.push((n, value));
        }
        // Restricted guarded-return sequence; every function still has a
        // final return. Fold guards into typed conditional expressions.
        let mut guards = Vec::new();
        while self.eat("if") {
            if guards.len() >= 16 {
                return Err("E_LINK_GUARD_LIMIT".into());
            }
            let condition = self.expr(0, 0, None)?;
            if self.eat(";") {
                self.need("do")?;
            } else {
                self.need("then")?;
            }
            self.need("return")?;
            let value = self.expr(0, 0, None)?;
            self.need(";")?;
            self.need("fi")?;
            guards.push((condition, value));
        }
        self.need("return")?;
        let mut result = self.expr(0, 0, None)?;
        self.need(";")?;
        self.need("}")?;
        for (condition, value) in guards.into_iter().rev() {
            result = Raw::If(Box::new(condition), Box::new(value), Box::new(result));
        }
        Ok(result)
    }
    fn scalar_type(&mut self) -> Result<ScalarType, String> {
        if self.eat("int") {
            Ok(ScalarType::Int)
        } else if self.eat("bool") {
            Ok(ScalarType::Bool)
        } else {
            Err("E_LINK_SCALAR_TYPE".into())
        }
    }
    fn parse(mut self) -> Result<Vec<Fn>, String> {
        let mut fns = Vec::new();
        while self.peek() != "<eof>" {
            let public = self.eat("pub");
            self.need("fnc")?;
            let name = self.name()?;
            self.need("(")?;
            let mut params = Vec::new();
            let mut param_types = Vec::new();
            if self.peek() != ")" {
                loop {
                    // The normative scalar declaration allows both
                    // type-first `int x` and name-first `x: int` formals.
                    // Never infer an omitted type in this native i64 ABI.
                    let (n, ty) = if matches!(self.peek(), "int" | "bool") {
                        let ty = self.scalar_type()?;
                        (self.name()?, ty)
                    } else {
                        let n = self.name()?;
                        self.need(":")?;
                        (n, self.scalar_type()?)
                    };
                    if params.contains(&n) {
                        return Err("E_PIPE_DUP_PARAM".into());
                    }
                    params.push(n);
                    param_types.push(ty);
                    if params.len() > self.max_arity {
                        return Err("E_PIPE_ARITY_LIMIT".into());
                    }
                    if !self.eat(",") {
                        break;
                    }
                }
            }
            self.need(")")?;
            let colon = self.eat(":");
            if !colon {
                self.need("->")?;
            }
            let return_type = self.scalar_type()?;
            let mut locals = Vec::new();
            let result = if self.eat("{") {
                self.body(&mut locals)?
            } else if colon && self.eat("->") {
                if self.eat("{") {
                    self.body(&mut locals)?
                } else {
                    let x = self.expr(0, 0, None)?;
                    self.need(";")?;
                    x
                }
            } else {
                return Err("E_PIPE_BODY".into());
            };
            fns.push(Fn {
                public,
                name,
                params,
                param_types,
                return_type,
                locals,
                result,
            });
            if fns.len() > 16 {
                return Err("E_PIPE_FUNCTION_LIMIT".into());
            }
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
    fn lowered(
        &mut self,
        raw: &Raw,
        env: &HashMap<String, Expr>,
        used: &mut HashMap<String, usize>,
        order: &mut Vec<String>,
        depth: usize,
    ) -> Result<Expr, String> {
        if depth > MAX_DEPTH || self.budget == 0 {
            return Err("E_PIPE_EXPANSION_LIMIT".into());
        }
        self.budget -= 1;
        Ok(match raw {
            Raw::NamedActual(_, _) => return Err("E_PIPE_NAMED_REQUIRES_LINKED_NATIVE".into()),
            Raw::Tuple(_) | Raw::TuplePattern(_, _, _) => {
                return Err("E_PIPE_TUPLE_REQUIRES_LINKED_NATIVE".into());
            }
            Raw::Literal(v) => Expr::Literal(*v),
            Raw::Bool(_)
            | Raw::Not(_)
            | Raw::Eq(_, _)
            | Raw::Ne(_, _)
            | Raw::BitAnd(_, _)
            | Raw::Lt(_, _)
            | Raw::Le(_, _)
            | Raw::Gt(_, _)
            | Raw::Ge(_, _)
            | Raw::And(_, _)
            | Raw::Or(_, _)
            | Raw::Xor(_, _)
            | Raw::If(_, _, _) => {
                return Err(
                    "E_PIPE_TYPE: typed Boolean/guard requires linked native lowering".into(),
                );
            }
            Raw::Name(n) => {
                let value = env.get(n).ok_or("E_PIPE_SYMBOL")?;
                *used.entry(n.clone()).or_default() += 1;
                order.push(n.clone());
                value.clone()
            }
            Raw::Add(a, b) => Expr::Add(
                Box::new(self.lowered(a, env, used, order, depth + 1)?),
                Box::new(self.lowered(b, env, used, order, depth + 1)?),
            ),
            Raw::Sub(a, b) => Expr::Sub(
                Box::new(self.lowered(a, env, used, order, depth + 1)?),
                Box::new(self.lowered(b, env, used, order, depth + 1)?),
            ),
            Raw::Mul(a, b) => Expr::Mul(
                Box::new(self.lowered(a, env, used, order, depth + 1)?),
                Box::new(self.lowered(b, env, used, order, depth + 1)?),
            ),
            Raw::Call(name, args) => {
                let mut params = Vec::new();
                for arg in args {
                    params.push(self.lowered(arg, env, used, order, depth + 1)?);
                }
                self.call(name, params, depth + 1)?
            }
        })
    }
    fn call(&mut self, name: &str, args: Vec<Expr>, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("E_PIPE_EXPANSION_LIMIT".into());
        }
        let func = self.functions.get(name).ok_or("E_PIPE_SYMBOL")?;
        if func.params.len() != args.len() {
            return Err("E_PIPE_ARITY".into());
        }
        if self.stack.iter().any(|s| s == name) {
            return Err("E_PIPE_CYCLE".into());
        }
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
            if env.contains_key(n) {
                return Err("E_PIPE_DUP_LOCAL".into());
            }
            let value = self.lowered(expr, &env, &mut used, &mut order, depth + 1)?;
            env.insert(n.clone(), value);
        }
        let answer = self.lowered(&func.result, &env, &mut used, &mut order, depth + 1)?;
        // Without a Cranelift let node or explicit function call IR, only
        // linearly consumed bindings are safe to substitute as pure trees.
        for n in func.params.iter().chain(func.locals.iter().map(|(n, _)| n)) {
            if used.get(n).copied() != Some(1) {
                return Err("E_PIPE_NONLINEAR_BINDING_UNSUPPORTED".into());
            }
        }
        // The call ABI evaluates argument expressions left-to-right before
        // entering the callee. Inlined parameter expressions must be observed
        // in their declaration order, or early checked failures could reorder.
        let parameter_uses: Vec<&str> = order
            .iter()
            .filter(|n| func.params.iter().any(|p| p == *n))
            .map(String::as_str)
            .collect();
        let declared_params: Vec<&str> = func.params.iter().map(String::as_str).collect();
        if parameter_uses != declared_params {
            return Err("E_PIPE_ARGUMENT_ORDER_UNSUPPORTED".into());
        }
        // Every eager val initializer must execute before later val bodies
        // and before the return expression. Enforce order of uses.
        let observed: Vec<&str> = order
            .iter()
            .filter(|n| func.locals.iter().any(|(declared, _)| declared == *n))
            .map(String::as_str)
            .collect();
        let declared: Vec<&str> = func.locals.iter().map(|(n, _)| n.as_str()).collect();
        if observed != declared {
            return Err("E_PIPE_LOCAL_ORDER_UNSUPPORTED".into());
        }
        Ok(answer)
    }
}
/// Compile a strict pure-i64 subset of named Atto functions by **checked**
/// static inlining, then let the existing Cranelift backend generate actual
/// machine code. This path is NOT an interpreter, general linker, or AOT.
pub fn parse(source: &str) -> Result<Program, String> {
    validate_source_envelope(source)?;
    compile_functions(Parser::new(source, 4)?.parse()?)
}

/// Compiler-only symbol: the source lexer cannot spell '$'.
pub(crate) const NAMED_STAGE: &str = "$closed_named_stage";
/// Admit only a closed scalar expression, and rewrite its lexical self
/// identity before the ordinary native call-graph checker sees the body.
fn close_named(e: &mut Raw, param: &str, own: &str, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if depth > 24 || *nodes > 128 { return Err("E_NAMED_STAGE_BUDGET".into()); }
    match e {
        Raw::Literal(_) => Ok(()),
        Raw::Name(n) if n == param => Ok(()),
        Raw::Add(a,b) | Raw::Sub(a,b) | Raw::Mul(a,b) => {
            close_named(a,param,own,depth+1,nodes)?;
            close_named(b,param,own,depth+1,nodes)
        }
        Raw::Call(n,args) if n == own && args.len()==1 => {
            close_named(&mut args[0],param,own,depth+1,nodes)?;
            *n=NAMED_STAGE.to_owned();
            Ok(())
        }
        _ => Err("E_NAMED_STAGE_CLOSED_INT".into()),
    }
}
/// Dedicated complete-file grammar. No fallback after token selection.
/// One public main, one literal upstream, one explicitly typed unary stage;
/// root match on the immutable parameter or a scalar return expression.
pub(crate) fn parse_named_recursive(source: &str) -> Result<Option<Vec<Fn>>, String> {
    validate_source_envelope(source)?;
    let tokens=match lex(source) { Ok(t)=>t, Err(_)=>return Ok(None) };
    if !tokens.windows(2).any(|w| w[0]=="|>" && w[1]=="fnc") { return Ok(None); }
    let mut p=Parser{tokens,at:0,max_arity:1};
    p.need("pub")?;p.need("fnc")?;p.need("main")?;p.need("(")?;p.need(")")?;
    if !p.eat(":"){p.need("->")?;}p.need("int")?;p.need("{")?;
    if !p.eat("return"){p.need("^")?;}
    let input=p.expr(6,0,None)?;
    if !matches!(input,Raw::Literal(_)) {return Err("E_NAMED_STAGE_LITERAL_UPSTREAM".into());}
    p.need("|>")?;p.need("fnc")?;
    let own=p.name()?;p.need("(")?;p.need("int")?;let param=p.name()?;p.need(")")?;
    if own==param || matches!(own.as_str(),"main"|"_"|"over"|"on"|"end"|"self"|"arguments"|"And"|"Or"|"Xor")
      || matches!(param.as_str(),"_"|"over"|"on"|"end"|"self"|"arguments") {
        return Err("E_NAMED_STAGE_NAME".into());
    }
    if !p.eat(":"){p.need("->")?;}p.need("int")?;p.need("{")?;
    if !p.eat("return"){p.need("^")?;}
    let body_start=p.at;
    let mut nodes=0;
    let result=if p.eat("match") {
        p.need(&param)?;p.need("over")?;
        let mut arms=Vec::new();
        let mut keys=std::collections::HashSet::new();
        let mut otherwise=None;
        while !p.eat("end") {
            if arms.len()>=16 || otherwise.is_some(){return Err("E_NAMED_STAGE_MATCH".into());}
            p.need("on")?;
            let key=if p.eat("_"){None}else{
                let minus=p.eat("-");
                let spelling=if minus{format!("-{}",p.peek())}else{p.peek().to_owned()};
                let value=spelling.parse::<i64>().map_err(|_|"E_NAMED_STAGE_PATTERN")?;
                p.at+=1;
                if !keys.insert(value){return Err("E_NAMED_STAGE_DUP_PATTERN".into());}
                Some(value)
            };
            p.need("->")?;
            let mut value=p.expr(0,0,None)?;
            close_named(&mut value,&param,&own,0,&mut nodes)?;
            if let Some(key)=key {arms.push((key,value));}else{otherwise=Some(value);}
            if !p.eat(",") && p.peek()!="end"{return Err("E_NAMED_STAGE_MATCH".into());}
        }
        let mut result=otherwise.ok_or("E_NAMED_STAGE_NONEXHAUSTIVE")?;
        for (key,body) in arms.into_iter().rev(){
            result=Raw::If(Box::new(Raw::Eq(Box::new(Raw::Name(param.clone())),Box::new(Raw::Literal(key)))),Box::new(body),Box::new(result));
        }
        result
    } else {
        let mut value=p.expr(0,0,None)?;
        close_named(&mut value,&param,&own,0,&mut nodes)?;
        value
    };
    // The general scalar expression parser may desugar pipelines into
    // calls. This profile admits only direct source self calls, not nested
    // pipelines, tuple patterns, or alternative callable forms.
    if p.tokens[body_start..p.at].iter().any(|t|matches!(t.as_str(),"|>"|"<|"|"|")){
        return Err("E_NAMED_STAGE_NESTED_PIPE".into());
    }
    p.eat(";");p.need("}")?;p.eat(";");p.need("}")?;
    if p.peek()!="<eof>" {return Err("E_NAMED_STAGE_TRAILING".into());}
    Ok(Some(vec![
        Fn{public:true,name:"main".into(),params:vec![],param_types:vec![],return_type:ScalarType::Int,locals:vec![],result:Raw::Call(NAMED_STAGE.into(),vec![input])},
        Fn{public:false,name:NAMED_STAGE.into(),params:vec![param],param_types:vec![ScalarType::Int],return_type:ScalarType::Int,locals:vec![],result},
    ]))
}

/// Preserve the separately parsed function graph and symbolic calls for
/// real native function lowering. This path never substitutes argument trees.
pub(crate) fn parse_linked_native(
    entry: &str,
    units: &[(String, String, String)],
) -> Result<Vec<Fn>, String> {
    if units.len() > 8 {
        return Err("E_LINK_IMPORT_LIMIT".into());
    }
    validate_source_envelope(entry)?;
    let mut selected = Parser::new(entry, LINKED_MAX_ARITY)?.parse()?;
    for (requested, local, text) in units {
        if reserved_sum_name(requested) || reserved_sum_name(local) {
            return Err("E_PIPE_TAGGED_SUM_UNSUPPORTED: typed Option/Result/trap is unsupported by the Cranelift scalar backend".into());
        }
        validate_source_envelope(text)?;
        let candidates = Parser::new(text, LINKED_MAX_ARITY)?.parse()?;
        let mut exported = candidates
            .into_iter()
            .filter(|f| f.name == *requested && f.public);
        let mut func = exported
            .next()
            .ok_or("E_LINK_EXPORT_NOT_PUBLIC_OR_MISSING")?;
        if exported.next().is_some() {
            return Err("E_LINK_AMBIGUOUS_EXPORT".into());
        }
        if func.name == "main" || local == "main" {
            return Err("E_LINK_IMPORTED_ENTRY".into());
        }
        // Rename the selected symbol in this closed-world linked scope only.
        // Private and unimported declarations stay unreachable.
        func.name = local.clone();
        selected.push(func);
    }
    Ok(selected)
}

fn compile_functions(functions: Vec<Fn>) -> Result<Program, String> {
    if functions.len() > 16 {
        return Err("E_PIPE_FUNCTION_LIMIT".into());
    }
    let mut names = HashMap::new();
    for f in functions {
        if f.return_type != ScalarType::Int || f.param_types.iter().any(|t| *t != ScalarType::Int) {
            return Err("E_PIPE_TYPE: typed callable requires linked native lowering".into());
        }
        if names.insert(f.name.clone(), f).is_some() {
            return Err("E_PIPE_DUP_FUNCTION".into());
        }
    }
    let main = names.get("main").ok_or("E_PIPE_ENTRY")?;
    if !main.params.is_empty() {
        return Err("E_PIPE_ENTRY".into());
    }
    // Also check dormant function bodies; reject errors before CLIF output.
    let mut sorted_names: Vec<_> = names.keys().cloned().collect();
    sorted_names.sort();
    for name in &sorted_names {
        let f = &names[name];
        let args = f.params.iter().map(|_| Expr::Literal(0)).collect();
        let mut lower = Lower {
            functions: &names,
            budget: MAX_EXPANSION,
            stack: Vec::new(),
        };
        lower.call(name, args, 0)?;
    }
    let mut lower = Lower {
        functions: &names,
        budget: MAX_EXPANSION,
        stack: Vec::new(),
    };
    let result = lower.call("main", Vec::new(), 0)?;
    validate_ast_depth(&result)?;
    Ok(Program {
        parameters: Vec::new(),
        locals: Vec::new(),
        result,
        trace_steps: Vec::new(),
    })
}
