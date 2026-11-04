//! Fail-closed, bounded *subset* of Oreslang. Not a general Oreslang parser.
const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 128;
const MAX_SOURCE: usize = 1024 * 1024;
const MAX_PARAMS: usize = 4;

// Historical Java v1 Lexer.java keyword inventory (93 spellings), pinned to
// Git blob bba9e091c0ad1c9b75e0c61e9dd173fcc18a236a.
// This only rejects reserved names in the narrow scalar parameter grammar.
// It does not claim parity of native lexer tokens, parser, or guest runtime.
const JAVA_V1_RESERVED_PARAMETER_NAMES: [&str; 93] = [
    "abstract",
    "actor",
    "arr",
    "as",
    "async",
    "await",
    "block",
    "break",
    "catch",
    "cb",
    "class",
    "conforms",
    "const",
    "continue",
    "contract",
    "cpu",
    "def",
    "default",
    "defer",
    "define",
    "do",
    "done",
    "elif",
    "else",
    "elseif",
    "end",
    "eq",
    "extends",
    "false",
    "fi",
    "finally",
    "fnc",
    "for",
    "from",
    "generator",
    "gpu",
    "if",
    "impl",
    "implements",
    "import",
    "interface",
    "is",
    "isoactor",
    "let",
    "loop",
    "match",
    "matches",
    "module",
    "mut",
    "namespace",
    "nb",
    "neq",
    "new",
    "nlex",
    "null",
    "obj",
    "of",
    "over",
    "panic",
    "private",
    "pub",
    "quantum",
    "raise",
    "readch",
    "recover",
    "return",
    "routine",
    "rt",
    "select",
    "self",
    "shared",
    "spawn",
    "static",
    "stop",
    "struct",
    "super",
    "suspends",
    "switch",
    "then",
    "throw",
    "trait",
    "trap",
    "true",
    "try",
    "type",
    "typeof",
    "types",
    "untrusted",
    "val",
    "void",
    "while",
    "writech",
    "yield",
];

fn is_java_v1_reserved_parameter_name(name: &str) -> bool {
    JAVA_V1_RESERVED_PARAMETER_NAMES
        .binary_search(&name)
        .is_ok()
}

fn integer_digits(token: &str) -> Result<String, String> {
    let bytes = token.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'_'
            && (i == 0
                || i + 1 == bytes.len()
                || !bytes[i - 1].is_ascii_digit()
                || !bytes[i + 1].is_ascii_digit())
        {
            return Err("invalid i64 digit separator".into());
        }
    }
    Ok(token.replace('_', ""))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Literal(i64),
    Param(usize),
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    BitXor(Box<Expr>, Box<Expr>),
}

#[derive(Debug)]
pub struct Program {
    pub parameters: Vec<String>,
    pub result: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token<'a> {
    Word(&'a str),
    Number(&'a str),
    Punct(u8),
    End,
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    current: Token<'a>,
    nodes: usize,
    parameters: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Result<Self, String> {
        if input.len() > MAX_SOURCE {
            return Err("source exceeds 1 MiB limit".into());
        }
        if input.as_bytes().contains(&0) {
            return Err("source contains NUL byte".into());
        }
        let mut p = Self {
            input,
            pos: 0,
            current: Token::End,
            nodes: 0,
            parameters: Vec::new(),
        };
        p.next()?;
        Ok(p)
    }

    fn next(&mut self) -> Result<(), String> {
        let b = self.input.as_bytes();
        loop {
            while self.pos < b.len() && b" \t\r\n\x0b\x0c".contains(&b[self.pos]) {
                self.pos += 1;
            }
            if b.get(self.pos..self.pos.saturating_add(2)) == Some(&b"//"[..]) {
                self.pos += 2;
                // CR-only files are legal source input, so a line comment
                // must not swallow the following model declaration.
                while self.pos < b.len() && b[self.pos] != b'\n' && b[self.pos] != b'\r' {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
        if self.pos == b.len() {
            self.current = Token::End;
            return Ok(());
        }
        let start = self.pos;
        let first = b[self.pos];
        self.pos += 1;
        // Stage-0 cannot bind injected state lifetimes or borrow authority.
        // Surface a deliberate diagnostic before any source can be compiled.
        // Inspect only lexed source, so comment text cannot trigger admission.
        if first == b'@' {
            let mut end = self.pos;
            while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
                end += 1;
            }
            let name = &self.input[self.pos..end];
            if matches!(
                name,
                "Inject" | "StateBacked" | "MutableStateContainer" | "StateContainer"
            ) {
                return Err(format!(
                    "STATE_INJECTION_UNSUPPORTED: @{name} requires owned state, typed borrow checking and lifecycle lowering"
                ));
            }
            return Err(format!(
                "ANNOTATION_UNSUPPORTED: @{name} requires typed annotation lowering"
            ));
        }
        if first.is_ascii_alphabetic() || first == b'_' {
            while self.pos < b.len() && (b[self.pos].is_ascii_alphanumeric() || b[self.pos] == b'_')
            {
                self.pos += 1;
            }
            self.current = Token::Word(&self.input[start..self.pos]);
            if self.current == Token::Word("unsafe") {
                return Err("unsafe source is not supported".into());
            }
            // Not a semantic type rejection or a successful sum-type compile:
            // the scalar-only Cranelift backend cannot represent tagged
            // Option/Result payloads or classify guest trap control effects.
            // Refuse these reserved constructs before AST/native codegen.
            if matches!(
                self.current,
                Token::Word("Option")
                    | Token::Word("Result")
                    | Token::Word("Some")
                    | Token::Word("None")
                    | Token::Word("Ok")
                    | Token::Word("Err")
                    | Token::Word("trap")
            ) {
                return Err("typed Option/Result/trap is unsupported by the Cranelift scalar backend; requires verified tagged sum and guest control lowering".into());
            }
        } else if first.is_ascii_digit() {
            while self.pos < b.len() && (b[self.pos].is_ascii_digit() || b[self.pos] == b'_') {
                self.pos += 1;
            }
            self.current = Token::Number(&self.input[start..self.pos]);
        } else if first == b'@' {
            // A recognized injection annotation is NOT source support.
            // Explicitly fail closed before any native JIT emission rather than
            // accidentally ignoring state-region ownership semantics.
            let remainder = &b[self.pos..];
            let intrinsic = b"Inject";
            let exact = remainder.starts_with(intrinsic)
                && remainder
                    .get(intrinsic.len())
                    .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != b'_');
            if exact {
                return Err("@Inject state-container lowering requires verified private regions, scoped borrows and runtime admission; unsupported in Cranelift".into());
            }
            return Err(
                "source annotations require checked compiler lowering; unsupported in Cranelift"
                    .into(),
            );
        } else if b"()+-*{};:,>.".contains(&first) {
            self.current = Token::Punct(first);
        } else {
            return Err(format!("invalid character at byte {start}"));
        }
        Ok(())
    }

    fn take(&mut self, t: Token<'a>) -> Result<bool, String> {
        if self.current == t {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn expect(&mut self, t: Token<'a>) -> Result<(), String> {
        if self.take(t)? {
            Ok(())
        } else {
            Err(format!("unexpected token at byte {}", self.pos))
        }
    }
    fn name(&mut self) -> Result<String, String> {
        match self.current {
            Token::Word(s) => {
                let out = s.to_owned();
                self.next()?;
                Ok(out)
            }
            _ => Err("expected identifier".into()),
        }
    }
    fn node(&mut self, x: Expr) -> Result<Expr, String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("AST node budget exceeded".into());
        }
        Ok(x)
    }
    fn atom(&mut self, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("expression depth budget exceeded".into());
        }
        if self.take(Token::Punct(b'-'))? {
            if let Token::Number(n) = self.current {
                // Special-case the only negative literal outside positive i64.
                if integer_digits(n)? == "9223372036854775808" {
                    self.next()?;
                    return self.node(Expr::Literal(i64::MIN));
                }
            }
            let rhs = self.atom(depth + 1)?;
            return self.node(Expr::Neg(Box::new(rhs)));
        }
        if self.take(Token::Punct(b'+'))? {
            return self.atom(depth + 1);
        }
        if self.take(Token::Punct(b'('))? {
            let e = self.expr(0, depth + 1)?;
            self.expect(Token::Punct(b')'))?;
            return Ok(e);
        }
        match self.current {
            Token::Number(n) => {
                let value = integer_digits(n)?
                    .parse::<i64>()
                    .map_err(|_| "i64 literal out of range")?;
                self.next()?;
                self.node(Expr::Literal(value))
            }
            Token::Word("Integer") => {
                // Recognize only this typed intrinsic. Unknown Integer.* calls
                // must reject before emitting native code.
                self.next()?;
                self.expect(Token::Punct(b'.'))?;
                self.expect(Token::Word("bitXor"))?;
                self.expect(Token::Punct(b'('))?;
                let lhs = self.expr(0, depth + 1)?;
                self.expect(Token::Punct(b','))?;
                let rhs = self.expr(0, depth + 1)?;
                self.expect(Token::Punct(b')'))?;
                self.node(Expr::BitXor(Box::new(lhs), Box::new(rhs)))
            }
            Token::Word("rt") => {
                // Stage-0 has no SharedMutex<T>, shared allocation domain, or
                // synchronized guard lowering. Never reinterpret this as a
                // scalar alias (or claim LLVM/Java guarded-mutation parity).
                self.next()?;
                if self.take(Token::Word("mut"))? && self.current == Token::Word("share") {
                    return Err("native rt mut share requires verified SharedMutex<T> capability, guarded lowering and allocation-domain proof; refusing unsynchronized alias".into());
                }
                Err("rt ownership operation requires checked ownership lowering".into())
            }
            Token::Word("io") => {
                Err("actor-local io.* requires native actor IR/ABI, generation-fenced ActorContext and bounded async stream lowering; Cranelift stage-0 refuses to emit native code".into())
            }
            Token::Word("stdio") => {
                Err("process stdio is outside the Cranelift stage-0 scalar profile; no actor/process I/O alias is permitted".into())
            }
            Token::Word("print") => {
                Err("process print is outside the Cranelift stage-0 scalar profile; no implicit console fallback is permitted".into())
            }
            Token::Word(s) => {
                let index = self
                    .parameters
                    .iter()
                    .position(|p| p == s)
                    .ok_or_else(|| format!("unbound or unsupported identifier: {s}"))?;
                self.next()?;
                self.node(Expr::Param(index))
            }
            _ => Err(format!("expected scalar expression at byte {}", self.pos)),
        }
    }
    fn expr(&mut self, min_bp: u8, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("expression depth budget exceeded".into());
        }
        let mut lhs = self.atom(depth + 1)?;
        loop {
            let (l_bp, r_bp, op) = match self.current {
                Token::Punct(b'+') => (1, 2, b'+'),
                Token::Punct(b'-') => (1, 2, b'-'),
                Token::Punct(b'*') => (3, 4, b'*'),
                _ => break,
            };
            if l_bp < min_bp {
                break;
            }
            self.next()?;
            let rhs = self.expr(r_bp, depth + 1)?;
            lhs = self.node(match op {
                b'+' => Expr::Add(Box::new(lhs), Box::new(rhs)),
                b'-' => Expr::Sub(Box::new(lhs), Box::new(rhs)),
                b'*' => Expr::Mul(Box::new(lhs), Box::new(rhs)),
                _ => unreachable!(),
            })?;
        }
        Ok(lhs)
    }
    fn program(mut self) -> Result<Program, String> {
        let mut seen_define = false;
        let mut seen_pub = false;
        loop {
            if self.take(Token::Word("define"))? {
                if seen_define {
                    return Err("duplicate define modifier".into());
                }
                seen_define = true;
            } else if self.take(Token::Word("pub"))? {
                if seen_pub {
                    return Err("duplicate pub modifier".into());
                }
                seen_pub = true;
            } else {
                break;
            }
        }
        // Stage-0 has no nominal type descriptors, sealed-state loans,
        // actor identity or supervisor CONTROL lane. A declaration must never
        // be reinterpreted as a scalar main() function.
        let unsupported_kind = match self.current {
            Token::Word("state") => Some("state container"),
            Token::Word(
                kind @ ("actor" | "supervisor" | "singleton" | "grain" | "model" | "enum" | "class"
                | "struct" | "record" | "interface" | "trait" | "protocol" | "service"),
            ) => Some(kind),
            _ => None,
        };
        if let Some(kind) = unsupported_kind {
            return Err(format!(
                "DECL_KIND_UNSUPPORTED: {kind} requires nominal/ownership lowering; Cranelift stage-0 supports only scalar fnc main"
            ));
        }
        if matches!(self.current, Token::Word("pattern") | Token::Word("match")) {
            return Err("native type-only ADT pattern lowering unsupported".into());
        }
        self.expect(Token::Word("fnc"))?;
        self.expect(Token::Word("main"))?;
        self.expect(Token::Punct(b'('))?;
        if !self.take(Token::Punct(b')'))? {
            loop {
                let type_first = self.take(Token::Word("int"))?;
                let name = self.name()?;
                if is_java_v1_reserved_parameter_name(&name)
                    || matches!(
                        name.as_str(),
                        "Integer"
                            | "int"
                            | "return"
                            | "pub"
                            | "define"
                            | "fnc"
                            | "rt"
                            | "async"
                            | "await"
                            | "actor"
                            | "isoactor"
                            | "panic"
                            | "raise"
                            | "throw"
                            | "recover"
                            | "defer"
                            | "spawn"
                            | "stop"
                            | "select"
                            | "match"
                            | "switch"
                            | "suspend"
                            | "let"
                            | "val"
                            | "const"
                            | "final"
                            | "share"
                            | "borrow"
                            | "take"
                            | "copy"
                            | "gpu"
                            | "cpu"
                            | "io"
                            | "stdio"
                            | "print"
                    )
                    || self.parameters.contains(&name)
                {
                    return Err("invalid or duplicate parameter".into());
                }
                if !type_first {
                    self.expect(Token::Punct(b':'))?;
                    self.expect(Token::Word("int"))?;
                }
                if self.parameters.len() == MAX_PARAMS {
                    return Err("parameter count exceeds four".into());
                }
                self.parameters.push(name);
                if !self.take(Token::Punct(b','))? {
                    break;
                }
            }
            self.expect(Token::Punct(b')'))?;
        }
        if !self.take(Token::Punct(b':'))? {
            self.expect(Token::Punct(b'-'))?;
            self.expect(Token::Punct(b'>'))?;
        }
        self.expect(Token::Word("int"))?;
        self.expect(Token::Punct(b'{'))?;
        self.expect(Token::Word("return"))?;
        let result = self.expr(0, 0)?;
        self.expect(Token::Punct(b';'))?;
        self.expect(Token::Punct(b'}'))?;
        self.expect(Token::End)?;
        Ok(Program {
            parameters: self.parameters,
            result,
        })
    }
}

pub fn parse(text: &str) -> Result<Program, String> {
    Parser::new(text)?.program()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_java_v1_reserved_parameter_identifiers_fail_closed() {
        assert_eq!(JAVA_V1_RESERVED_PARAMETER_NAMES.len(), 93);
        assert!(
            JAVA_V1_RESERVED_PARAMETER_NAMES
                .windows(2)
                .all(|p| p[0] < p[1])
        );
        for name in JAVA_V1_RESERVED_PARAMETER_NAMES {
            let source = format!("fnc main({name}:int):int{{return {name};}}");
            assert!(
                parse(&source).is_err(),
                "reserved Java v1 name accepted: {name}"
            );
        }
        for name in [
            "owned",
            "eqx",
            "xneq",
            "island",
            "nullish",
            "panic_mode",
            "cpu_task",
        ] {
            let source = format!("fnc main({name}:int):int{{return {name};}}");
            assert!(parse(&source).is_ok(), "ordinary name rejected: {name}");
        }
    }

    #[test]
    fn stage_zero_is_not_a_general_java_lexical_frontend() {
        for expression in [
            "#ready",
            "#'with spaces'",
            "critical-read",
            "transform-pipe",
            "nb-select",
            "gpu",
            "cpu",
            "panic",
            "1 ^ 2",
        ] {
            let source = format!("fnc main():int{{return {expression};}}");
            assert!(
                parse(&source).is_err(),
                "unsupported source admitted: {expression}"
            );
        }
        // Retired percent syntax remains rejected after mainline operator removal.
        assert!(parse("fnc main():int{return 7%3;}").is_err());
    }

    fn eval(e: &Expr, args: &[i64]) -> Result<i64, &'static str> {
        match e {
            Expr::Literal(x) => Ok(*x),
            Expr::Param(i) => Ok(args[*i]),
            Expr::Neg(x) => eval(x, args)?.checked_neg().ok_or("overflow"),
            Expr::Add(a, b) => eval(a, args)?.checked_add(eval(b, args)?).ok_or("overflow"),
            Expr::Sub(a, b) => eval(a, args)?.checked_sub(eval(b, args)?).ok_or("overflow"),
            Expr::Mul(a, b) => eval(a, args)?.checked_mul(eval(b, args)?).ok_or("overflow"),
            Expr::BitXor(a, b) => Ok(eval(a, args)? ^ eval(b, args)?),
        }
    }
    #[test]
    fn model_declarations_fail_closed_without_reserving_model_identifiers() {
        for source in [
            "define model x as Identity() end",
            "define model PriceEstimate() as Identity() end",
            "define pub model x as Identity() end",
            "pub define model x as Identity() end",
            "define // comment\nmodel x as Identity() end",
            "define // comment\rmodel x as Identity() end",
            "define // comment\r\nmodel x as Identity() end",
        ] {
            let error = parse(source).unwrap_err();
            assert!(
                error.contains("DECL_KIND_UNSUPPORTED: model"),
                "wrong admission diagnostic for {source}: {error}"
            );
        }
        let ordinary = parse("fnc main(model:int):int{return model+1;}").unwrap();
        assert_eq!(eval(&ordinary.result, &[41]), Ok(42));
        let commented = parse("fnc main():int{return 42;} // define model Fake").unwrap();
        assert_eq!(eval(&commented.result, &[]), Ok(42));
    }
    #[test]
    fn precedence_and_arguments() {
        let p = parse("define pub fnc main(x:int,y:int):int{return (x+2)*y-1;}").unwrap();
        assert_eq!(eval(&p.result, &[3, 4]), Ok(19));
        assert_eq!(
            parse("fnc main():int{return 2+3*4;}")
                .map(|p| eval(&p.result, &[]).unwrap())
                .unwrap(),
            14
        );
    }
    #[test]
    fn integer_edges() {
        let min = parse("fnc main():int{return -9223372036854775808;}").unwrap();
        assert_eq!(eval(&min.result, &[]), Ok(i64::MIN));
        assert!(parse("fnc main():int{return 9223372036854775808;}").is_err());
        let under = parse("fnc main():int{return --9223372036854775808;}").unwrap();
        assert!(eval(&under.result, &[]).is_err());
    }
    #[test]
    fn guest_bitxor_parsing_and_eager_eval() {
        let program = parse("fnc main(x:int,y:int):int{return Integer.bitXor(x+1,y*2);}").unwrap();
        assert_eq!(eval(&program.result, &[4, 3]), Ok(3));
        let edge = parse(
            "fnc main():int{return Integer.bitXor(-9223372036854775808,9223372036854775807);}",
        )
        .unwrap();
        assert_eq!(eval(&edge.result, &[]), Ok(-1));
    }
    #[test]
    fn removed_remainder_operator_is_rejected_before_lowering() {
        // The language contract removed %, ^ and ^^. The tokenizer must
        // reject all forms of % rather than accepting a restricted subset.
        for source in [
            "fnc main():int{return -7%3;}",
            "fnc main():int{return 7%-3;}",
            "fnc main():int{return -7%-3;}",
            "fnc main():int{return 8%3*2;}",
            "fnc main():int{return 20%6%3;}",
            "fnc main():int{return -9223372036854775808%-1;}",
            "fnc main(x:int):int{return x%0;}",
            "fnc main(x:int,y:int):int{return x%y;}",
            "fnc main(x:int):int{return x%(1+1);}",
            "fnc main(x:int):int{return x%-x;}",
        ] {
            assert!(
                parse(source).is_err(),
                "removed operator unexpectedly parsed: {source}"
            );
        }
        // Comments are non-executable text and can contain a percent sign.
        assert!(parse("fnc main():int{return 42;} // 100% comment").is_ok());
    }
    #[test]
    fn option_result_trap_fail_closed_until_native_tagged_lowering() {
        // Only the scalar contract is executable; these are unsupported
        // features, NOT passing semantic Option/Result/trap conformance cases.
        for source in [
            "fnc main():Option<int>{return Some(7);}",
            "fnc main():Option<int>{return None;}",
            "fnc main():Result<int,int>{return Ok(7);}",
            "fnc main():Result<int,int>{return Err(7);}",
            "trap fnc main():int{return 1;}",
            "fnc main():int{return Some(7);}",
            "fnc main():int{return Ok(7);}",
            "fnc main():int{return Err(7);}",
            "fnc main():int{return None;}",
            "fnc main():int{return Option(7);}",
            "fnc main():int{return Result(7);}",
        ] {
            let error = parse(source).expect_err("unsupported sum/trap must reject");
            assert!(
                error.contains("typed Option/Result/trap is unsupported"),
                "unexpected fail-open or error category for {source}: {error}"
            );
        }
        assert!(parse("fnc main():int{return 7;} // Option Result trap").is_ok());
    }
    #[test]
    fn actor_control_ownership_keywords_are_not_scalar_parameters() {
        for keyword in [
            "rt", "panic", "raise", "throw", "recover", "defer", "spawn", "stop", "select",
            "match", "switch", "suspend", "share", "borrow", "take", "copy", "gpu", "cpu",
            "isoactor", "let", "val", "const", "final",
        ] {
            let source = format!("fnc main({keyword}:int):int{{return {keyword};}}");
            assert!(
                parse(&source).is_err(),
                "{keyword} rebound into scalar parameter"
            );
        }
        // Valid snake_case identifiers containing reserved substrings remain legal.
        for name in ["rt_count", "panic_count", "spawn_count", "gpu_cycles"] {
            let source = format!("fnc main({name}:int):int{{return {name};}}");
            let parsed = parse(&source).unwrap();
            assert_eq!(eval(&parsed.result, &[42]), Ok(42));
        }
        let ownership = parse("fnc main():int{return rt mut share;}").unwrap_err();
        assert!(ownership.contains("SharedMutex"), "{ownership}");
    }

    #[test]
    fn actor_io_namespace_is_explicitly_fail_closed() {
        for source in [
            "fnc main():int{return io;}",
            "fnc main():int{return io.out;}",
            "fnc main():int{return stdio;}",
            "fnc main():int{return print;}",
        ] {
            let error = parse(source).expect_err(source);
            assert!(
                error.contains("actor-local io.*")
                    || error.contains("process stdio")
                    || error.contains("process print"),
                "{source}: wrong actor-I/O diagnostic: {error}"
            );
        }
        let actor = parse(
            "define actor Worker as { receive(ActorMail<int> mail):void { return; } } fnc main():int{return 0;}"
        ).expect_err("actor source must be rejected");
        assert!(
            !actor.trim().is_empty(),
            "actor source rejection must carry a diagnostic"
        );
    }

    #[test]
    fn fail_closed() {
        for source in [
            "fnc main():int{return 1/0;}",
            "fnc main():int{return 5 ^ 3;}",
            "fnc main():int{return Integer.rem(5,3);}",
            "fnc main():int{return Integer.bitXor(5);}",
            "fnc main():int{return Integer.bitXor(5,3,1);}",
            "fnc main(Integer:int):int{return Integer;}",
            "fnc main():float{return 1;}",
            "fnc main():int{return x;}",
            "fnc main():int{return 1;} fnc foo():int{return 2;}",
            "fnc main():int{return 1;} /*no*/",
            "fnc main():int{return 1.0;}",
            "fnc main(x:int,x:int):int{return x;}",
            "fnc main():int{return 1;} unsafe",
            "fnc main():int{return 1;}\0",
            "fnc main():int{return 1;}\u{00a0}",
            "fnc main(io:int):int{return io;}",
            "fnc main(stdio:int):int{return stdio;}",
            "fnc main(print:int):int{return print;}",
        ] {
            assert!(parse(source).is_err(), "{source:?}");
        }
        assert!(parse(&format!("fnc main():int{{return {};}}", "(".repeat(150))).is_err());
        assert!(parse(&" ".repeat(MAX_SOURCE + 1)).is_err());
        assert!(parse("fnc main():int{return 42;} // café").is_ok());
    }
}
