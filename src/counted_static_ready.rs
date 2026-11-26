//! Literal or singleton-proven count closed-world scalarization into the existing native
//! Cranelift Program IR. Queue readiness/order is statically known, but every
//! payload, projection and returned expression executes as checked native code.
//! No external producer, unproven dynamic count, actor or scheduler claim.
use crate::frontend::{Expr, Program};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

type R<T> = Result<T, String>;
fn err(s: impl AsRef<str>) -> String {
    format!("E_COUNTED_STATIC_READY: {}", s.as_ref())
}
fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}
fn lit(n: i64) -> Expr {
    Expr::Literal(n)
}
fn choose(c: Expr, y: Expr, n: Expr) -> Expr {
    Expr::Choose(boxed(c), boxed(y), boxed(n))
}
#[derive(Clone)]
enum Slot {
    Scalar(usize, bool),
    Channel { cap: usize, queue: VecDeque<usize> },
    Future { value: Expr, retired: bool },
    Array(Vec<usize>),
    Pending(Plan),
    Consumed,
}
#[derive(Clone)]
struct Arm {
    source: String,
    projection: Expr,
}
#[derive(Clone)]
struct Plan {
    count: usize,
    fair: bool,
    arms: Vec<Arm>,
}
enum Value {
    Int(Expr),
    Bool(Expr),
}
impl Value {
    fn int(self) -> R<Expr> {
        match self {
            Self::Int(x) => Ok(x),
            _ => Err(err("expected int")),
        }
    }
    fn boolean(self) -> R<Expr> {
        match self {
            Self::Bool(x) => Ok(x),
            _ => Err(err("expected bool")),
        }
    }
}
fn project(e: &Expr, slot: usize) -> R<Expr> {
    let pair = |a: &Expr, b: &Expr| -> R<(Box<Expr>, Box<Expr>)> {
        Ok((boxed(project(a, slot)?), boxed(project(b, slot)?)))
    };
    Ok(match e {
        Expr::Local(i) if *i == usize::MAX => Expr::Local(slot),
        Expr::Literal(v) => lit(*v),
        Expr::Neg(x) => Expr::Neg(boxed(project(x, slot)?)),
        Expr::Add(a, b) => {
            let (a, b) = pair(a, b)?;
            Expr::Add(a, b)
        }
        Expr::Sub(a, b) => {
            let (a, b) = pair(a, b)?;
            Expr::Sub(a, b)
        }
        Expr::Mul(a, b) => {
            let (a, b) = pair(a, b)?;
            Expr::Mul(a, b)
        }
        _ => return Err(err("non-scalar projection")),
    })
}
struct Parser {
    tokens: Vec<String>,
    pos: usize,
    slots: BTreeMap<String, Slot>,
    locals: Vec<Expr>,
    pending: Option<String>,
    sites: usize,
    budget: usize,
    depth: usize,
}
impl Parser {
    fn new(src: &str) -> R<Self> {
        if src.len() > 65536
            || src
                .bytes()
                .any(|b| b >= 127 || (b < 32 && !matches!(b, b'\n' | b'\r' | b'\t')))
        {
            return Err(err("bounded ASCII source required"));
        }
        let raw = src.as_bytes();
        let mut tokens = vec![];
        let mut i = 0;
        while i < raw.len() {
            let b = raw[i];
            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            if raw[i..].starts_with(b"//") {
                while i < raw.len() && raw[i] != b'\n' {
                    i += 1
                }
                continue;
            }
            let start = i;
            i += 1;
            if b.is_ascii_alphabetic() || b == b'_' {
                while i < raw.len() && (raw[i].is_ascii_alphanumeric() || raw[i] == b'_') {
                    i += 1
                }
            } else if b.is_ascii_digit() {
                while i < raw.len() && raw[i].is_ascii_digit() {
                    i += 1
                }
            } else if i < raw.len()
                && matches!(
                    (b, raw[i]),
                    (b'-', b'>')
                        | (b'=', b'=')
                        | (b'!', b'=')
                        | (b'<', b'=')
                        | (b'>', b'=')
                        | (b'&', b'&')
                        | (b'|', b'|')
                )
            {
                i += 1
            } else if !b"(){}[]:;,+-*<>=.".contains(&b) {
                return Err(err("unsupported token"));
            }
            if i - start > 128 || tokens.len() >= 8192 {
                return Err(err("token budget"));
            }
            tokens.push(src[start..i].to_string());
        }
        tokens.push("<eof>".into());
        Ok(Self {
            tokens,
            pos: 0,
            slots: BTreeMap::new(),
            locals: vec![],
            pending: None,
            sites: 0,
            budget: 4096,
            depth: 0,
        })
    }
    fn peek(&self) -> &str {
        self.tokens
            .get(self.pos)
            .map(String::as_str)
            .unwrap_or("<eof>")
    }
    fn eat(&mut self, t: &str) -> bool {
        if self.peek() == t {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn need(&mut self, t: &str) -> R<()> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(err(format!("expected {t}")))
        }
    }
    fn name(&mut self) -> R<String> {
        let n = self.peek();
        if !n.as_bytes().first().is_some_and(|b|b.is_ascii_alphabetic()||*b==b'_')||!n.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')||
            "pub suspends fnc main int bool val let const Channel Future Array new writech readch select nb await when fair first random default over end return if then fi true false".split_whitespace().any(|x|x==n){return Err(err("lexical name required"))}
        let n = n.to_owned();
        self.pos += 1;
        Ok(n)
    }
    fn small(&mut self, max: usize) -> R<usize> {
        let n = self
            .peek()
            .parse::<usize>()
            .map_err(|_| err("literal count/index/capacity required"))?;
        if n > max {
            return Err(err("literal bound"));
        }
        self.pos += 1;
        Ok(n)
    }
    fn local(&mut self, e: Expr) -> R<usize> {
        if self.locals.len() >= 64 {
            return Err(err("native local budget"));
        }
        let i = self.locals.len();
        self.locals.push(e);
        Ok(i)
    }
    fn ty(&mut self) -> R<&'static str> {
        if self.eat("int") {
            return Ok("int");
        }
        if self.eat("Channel") {
            self.need("<")?;
            self.need("int")?;
            self.need(">")?;
            return Ok("channel");
        }
        if self.eat("Future") {
            self.need("<")?;
            if self.eat("Array") {
                self.need("<")?;
                self.need("int")?;
                self.need(">")?;
                self.need(">")?;
                return Ok("pending");
            }
            self.need("int")?;
            self.need(">")?;
            return Ok("future");
        }
        if self.eat("Array") {
            let b = self.eat("[");
            if !b {
                self.need("<")?
            }
            self.need("int")?;
            self.need(if b { "]" } else { ">" })?;
            return Ok("array");
        }
        Err(err("unsupported lexical type"))
    }
    fn prec(t: &str) -> u8 {
        match t {
            "||" => 1,
            "&&" => 2,
            "==" | "!=" => 3,
            "<" | ">" | "<=" | ">=" => 4,
            "+" | "-" => 5,
            "*" => 6,
            _ => 0,
        }
    }
    // mode: binder for pure projection/default; terminal permits only reads
    // and scalar Future observations, never batch creation.
    fn expr(&mut self, min: u8, binder: Option<&str>, terminal: bool) -> R<Value> {
        if self.budget == 0 || self.depth >= 64 {
            return Err(err("expression budget"));
        }
        self.budget -= 1;
        self.depth += 1;
        let mut lhs = self.primary(binder, terminal)?;
        loop {
            let p = Self::prec(self.peek());
            if p == 0 || p < min {
                break;
            }
            let op = self.peek().to_owned();
            self.pos += 1;
            if terminal && matches!(op.as_str(), "&&" | "||") {
                return Err(err("effectful short circuit not supported"));
            }
            let rhs = self.expr(p + 1, binder, terminal)?;
            lhs = match op.as_str() {
                "+" | "-" | "*" => {
                    let a = boxed(lhs.int()?);
                    let b = boxed(rhs.int()?);
                    Value::Int(match op.as_str() {
                        "+" => Expr::Add(a, b),
                        "-" => Expr::Sub(a, b),
                        _ => Expr::Mul(a, b),
                    })
                }
                "&&" => Value::Bool(choose(lhs.boolean()?, rhs.boolean()?, lit(0))),
                "||" => Value::Bool(choose(lhs.boolean()?, lit(1), rhs.boolean()?)),
                _ => Value::Bool(Expr::Compare(
                    match op.as_str() {
                        "==" => 0,
                        "!=" => 1,
                        "<" => 2,
                        "<=" => 3,
                        ">" => 4,
                        ">=" => 5,
                        _ => return Err(err("operator")),
                    },
                    boxed(lhs.int()?),
                    boxed(rhs.int()?),
                )),
            };
        }
        self.depth -= 1;
        Ok(lhs)
    }
    fn primary(&mut self, binder: Option<&str>, terminal: bool) -> R<Value> {
        if self.eat("(") {
            let v = self.expr(0, binder, terminal)?;
            self.need(")")?;
            return Ok(v);
        }
        if self.eat("-") {
            return Ok(Value::Int(Expr::Neg(boxed(
                self.expr(7, binder, terminal)?.int()?,
            ))));
        }
        if self
            .peek()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
        {
            let n = self.peek().parse::<i64>().map_err(|_| err("i64 literal"))?;
            self.pos += 1;
            return Ok(Value::Int(lit(n)));
        }
        if let Some(b) = binder {
            if self.name()? != b {
                return Err(err("projection binder only"));
            }
            return Ok(Value::Int(Expr::Local(usize::MAX)));
        }
        if self.eat("readch") {
            if !terminal || self.pending.is_some() {
                return Err(err("read only in terminal return"));
            }
            let n = self.name()?;
            return match self.slots.get_mut(&n) {
                Some(Slot::Channel { queue, .. }) => Ok(Value::Int(Expr::Local(
                    queue.pop_front().ok_or_else(|| err("would suspend"))?,
                ))),
                _ => Err(err("expected Channel")),
            };
        }
        if self.eat("await") {
            if !terminal || self.pending.is_some() {
                return Err(err("await only in terminal return or array initializer"));
            }
            let n = self.name()?;
            return match self.slots.get_mut(&n) {
                Some(Slot::Future { value, retired }) if !*retired => {
                    *retired = true;
                    Ok(Value::Int(value.clone()))
                }
                _ => Err(err("expected live Future")),
            };
        }
        let n = self.name()?;
        match self.slots.get(&n).cloned() {
            Some(Slot::Scalar(i, _)) => Ok(Value::Int(Expr::Local(i))),
            Some(Slot::Array(a)) => {
                if self.eat("[") {
                    let i = self.small(15)?;
                    self.need("]")?;
                    Ok(Value::Int(Expr::Local(
                        *a.get(i).ok_or_else(|| err("array bounds"))?,
                    )))
                } else {
                    self.need(".")?;
                    self.need("size")?;
                    Ok(Value::Int(lit(a.len() as i64)))
                }
            }
            _ => Err(err("unknown or escaping capability")),
        }
    }
    // Abstract evaluation of the pure, checked integer subset. Never fold
    // emitted guest arithmetic away. Only a singleton count permits this
    // static topology profile; unknown or overflowing proofs fail closed.
    fn prove(&self, e: &Expr, fuel: &mut usize) -> Option<i64> {
        *fuel = fuel.checked_sub(1)?;
        match e {
            Expr::Literal(n) => Some(*n),
            Expr::Local(i) => self.prove(self.locals.get(*i)?, fuel),
            Expr::Neg(a) => self.prove(a, fuel)?.checked_neg(),
            Expr::Add(a, b) => self.prove(a, fuel)?.checked_add(self.prove(b, fuel)?),
            Expr::Sub(a, b) => self.prove(a, fuel)?.checked_sub(self.prove(b, fuel)?),
            Expr::Mul(a, b) => self.prove(a, fuel)?.checked_mul(self.prove(b, fuel)?),
            Expr::RefinementCast(a, lo, hi) => self.prove(a, fuel).filter(|n| lo <= n && n <= hi),
            _ => None,
        }
    }
    fn plan(&mut self) -> R<Plan> {
        if self.pending.is_some() {
            return Err(err("await outstanding batch first"));
        }
        if self.eat("<") {
            self.need("int")?;
            self.need(">")?
        }
        self.need("(")?;
        let value = self.expr(0, None, false)?.int()?;
        self.need(")")?;
        let proven = self
            .prove(&value, &mut 4096)
            .ok_or_else(|| err("count is not singleton-proven"))?;
        if !(0..=16).contains(&proven) {
            return Err(err("SelectCountInvalid"));
        }
        // Snapshot now, before a pending handle can escape or a let changes.
        // The native checked evaluation and exact refinement remain in IR.
        self.local(Expr::RefinementCast(boxed(value), proven, proven))?;
        let count = proven as usize;
        let fair = self.eat("fair");
        if !fair {
            self.eat("first");
        }
        let over = self.eat("over");
        if !over {
            self.need("{")?
        }
        let mut arms = vec![];
        while self.eat("when") {
            if arms.len() >= 8 {
                return Err(err("arm budget"));
            }
            let is_channel = self.eat("readch");
            if !is_channel {
                self.need("await")?
            }
            let source = self.name()?;
            match self.slots.get(&source) {
                Some(Slot::Channel { .. }) if is_channel => {}
                Some(Slot::Future { retired: false, .. }) if !is_channel => {}
                _ => return Err(err("wrong or retired source")),
            }
            self.need(":")?;
            self.need("int")?;
            let binder = self.name()?;
            self.need("->")?;
            self.need("{")?;
            self.need("return")?;
            let projection = self.expr(0, Some(&binder), false)?.int()?;
            self.need(";")?;
            self.need("}")?;
            arms.push(Arm { source, projection });
        }
        if arms.is_empty() {
            return Err(err("at least one arm required"));
        }
        self.need(if over { "end" } else { "}" })?;
        self.sites += 1;
        Ok(Plan { count, fair, arms })
    }
    fn drive(&mut self, p: Plan) -> R<Vec<usize>> {
        let mut out = vec![];
        let mut start = 0;
        let mut retired = BTreeSet::new();
        for _ in 0..p.count {
            let winner = (0..p.arms.len())
                .map(|offset| (start + offset) % p.arms.len())
                .find(|&i| {
                    let n = &p.arms[i].source;
                    match self.slots.get(n) {
                        Some(Slot::Channel { queue, .. }) => !queue.is_empty(),
                        Some(Slot::Future { retired: false, .. }) => !retired.contains(n),
                        _ => false,
                    }
                })
                .ok_or_else(|| err("statically known would-suspend/exhaustion"))?;
            let arm = &p.arms[winner];
            let source = self
                .slots
                .get_mut(&arm.source)
                .ok_or_else(|| err("source"))?;
            let payload = match source {
                Slot::Channel { queue, .. } => {
                    queue.pop_front().ok_or_else(|| err("empty queue"))?
                }
                Slot::Future {
                    value,
                    retired: done,
                } => {
                    *done = true;
                    retired.insert(arm.source.clone());
                    let e = value.clone();
                    self.local(e)?
                }
                _ => return Err(err("source")),
            };
            out.push(self.local(project(&arm.projection, payload)?)?);
            if p.fair {
                start = (winner + 1) % p.arms.len();
            }
        }
        // Positive counts move all Future witnesses, even if not selected.
        if p.count > 0 {
            for arm in &p.arms {
                if let Some(Slot::Future { retired, .. }) = self.slots.get_mut(&arm.source) {
                    *retired = true;
                }
            }
        }
        Ok(out)
    }
    fn bind(&mut self, mutable: bool) -> R<()> {
        let t = self.ty()?;
        let n = self.name()?;
        if self.slots.contains_key(&n) || self.slots.len() >= 64 {
            return Err(err("lexical binding quota"));
        }
        self.need("=")?;
        if mutable && t != "int" {
            return Err(err("only scalar mutation supported"));
        }
        let slot = match t {
            "channel" => {
                if self.pending.is_some() {
                    return Err(err("await batch first"));
                }
                self.need("Channel")?;
                self.need(".")?;
                self.need("new")?;
                self.need("<")?;
                self.need("int")?;
                self.need(">")?;
                self.need("(")?;
                let cap = self.small(64)?;
                self.need(")")?;
                if cap == 0 {
                    return Err(err("zero capacity"));
                }
                Slot::Channel {
                    cap,
                    queue: VecDeque::new(),
                }
            }
            "future" => {
                if self.pending.is_some() {
                    return Err(err("await batch first"));
                }
                self.need("select")?;
                self.need("<")?;
                self.need("int")?;
                self.need(">")?;
                let over = self.eat("over");
                if !over {
                    self.need("{")?
                }
                self.need("default")?;
                self.need(":")?;
                self.need("->")?;
                self.need("{")?;
                self.need("return")?;
                let value = self.expr(0, Some("$no_binder"), false)?.int()?;
                self.need(";")?;
                self.need("}")?;
                self.need(if over { "end" } else { "}" })?;
                Slot::Future {
                    value,
                    retired: false,
                }
            }
            "pending" => {
                self.eat("nb");
                self.need("select")?;
                let p = self.plan()?;
                self.pending = Some(n.clone());
                Slot::Pending(p)
            }
            "array" => {
                self.need("await")?;
                let legacy = self.eat("nb");
                let p = if self.eat("select") {
                    self.plan()?
                } else {
                    if legacy {
                        return Err(err("nb requires select"));
                    }
                    let h = self.name()?;
                    if self.pending.as_deref() != Some(&h) {
                        return Err(err("expected outstanding batch"));
                    }
                    let Some(Slot::Pending(p)) = self.slots.get(&h).cloned() else {
                        return Err(err("pending plan"));
                    };
                    self.slots.insert(h, Slot::Consumed);
                    self.pending = None;
                    p
                };
                Slot::Array(self.drive(p)?)
            }
            "int" => {
                let v = self.expr(0, None, false)?.int()?;
                Slot::Scalar(self.local(v)?, mutable)
            }
            _ => return Err(err("type")),
        };
        self.need(";")?;
        self.slots.insert(n, slot);
        Ok(())
    }
    fn parse(mut self) -> R<Program> {
        self.eat("pub");
        self.need("suspends")?;
        self.need("fnc")?;
        self.need("main")?;
        self.need("(")?;
        self.need(")")?;
        self.need(":")?;
        self.need("int")?;
        self.need("{")?;
        let mut conditions = vec![];
        let mut statements = 0;
        while self.peek() != "return" {
            statements += 1;
            if statements > 128 {
                return Err(err("statement quota"));
            }
            if self.eat("val") {
                if !conditions.is_empty() {
                    return Err(err("only terminal condition chain supported"));
                }
                self.bind(false)?
            } else if self.eat("let") {
                if !conditions.is_empty() {
                    return Err(err("only terminal condition chain supported"));
                }
                self.bind(true)?
            } else if self.tokens.get(self.pos + 1).is_some_and(|s| s == "=") {
                if !conditions.is_empty() {
                    return Err(err("assignment after terminal branch"));
                }
                let n = self.name()?;
                if !matches!(self.slots.get(&n), Some(Slot::Scalar(_, true))) {
                    return Err(err("immutable or unknown scalar"));
                }
                self.need("=")?;
                let v = self.expr(0, None, false)?.int()?;
                self.need(";")?;
                let i = self.local(v)?;
                self.slots.insert(n, Slot::Scalar(i, true));
            } else if self.eat("writech") {
                if self.pending.is_some() || !conditions.is_empty() {
                    return Err(err(
                        "write before outstanding await or after terminal branch",
                    ));
                }
                let n = self.name()?;
                self.need(",")?;
                let v = self.expr(0, None, false)?.int()?;
                self.need(";")?;
                let i = self.local(v)?;
                match self.slots.get_mut(&n) {
                    Some(Slot::Channel { cap, queue }) if queue.len() < *cap => queue.push_back(i),
                    _ => return Err(err("channel full or unavailable")),
                }
            } else if self.eat("if") {
                if self.pending.is_some() {
                    return Err(err("await outstanding batch first"));
                }
                let c = self.expr(0, None, false)?.boolean()?;
                self.need("then")?;
                self.need("return")?;
                let saved = self.slots.clone();
                let v = self.expr(0, None, true)?.int()?;
                self.slots = saved;
                self.need(";")?;
                self.need("fi")?;
                conditions.push((c, v));
            } else {
                return Err(err("unsupported statement or dynamic count"));
            }
        }
        self.need("return")?;
        let mut result = self.expr(0, None, true)?.int()?;
        self.need(";")?;
        self.need("}")?;
        if self.peek() != "<eof>" || self.sites == 0 || self.pending.is_some() {
            return Err(err("closed-world counted main required"));
        }
        for (c, v) in conditions.into_iter().rev() {
            result = choose(c, v, result)
        }
        Ok(Program {
            parameters: vec![],
            locals: self.locals,
            result,
            trace_steps: vec![],
        })
    }
}
pub fn compile(source: &str) -> R<Program> {
    Parser::new(source)?.parse()
}
