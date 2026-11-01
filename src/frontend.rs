//! Fail-closed, bounded *subset* of Oreslang. Not a general Oreslang parser.
const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 128;
const MAX_SOURCE: usize = 1024 * 1024;
const MAX_PARAMS: usize = 4;

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
    // The divisor is compiler-proven to be a nonzero signed i64 literal.
    Rem(Box<Expr>, i64),
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
                while self.pos < b.len() && b[self.pos] != b'\n' {
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
        if first.is_ascii_alphabetic() || first == b'_' {
            while self.pos < b.len() && (b[self.pos].is_ascii_alphanumeric() || b[self.pos] == b'_')
            {
                self.pos += 1;
            }
            self.current = Token::Word(&self.input[start..self.pos]);
            if self.current == Token::Word("unsafe") {
                return Err("unsafe source is not supported".into());
            }
        } else if first.is_ascii_digit() {
            while self.pos < b.len() && (b[self.pos].is_ascii_digit() || b[self.pos] == b'_') {
                self.pos += 1;
            }
            self.current = Token::Number(&self.input[start..self.pos]);
        } else if b"()+-*%{};:,>.".contains(&first) {
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
                Token::Punct(b'%') => (3, 4, b'%'),
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
                b'%' => {
                    // Stage-0 parity with LLVM's literal-divisor remainder:
                    // never lower dynamic or zero divisors into native JIT code.
                    let divisor = match rhs {
                        Expr::Literal(value) => value,
                        Expr::Neg(inner) => match *inner {
                            Expr::Literal(value) => value
                                .checked_neg()
                                .ok_or("remainder divisor out of range")?,
                            _ => {
                                return Err(
                                    "remainder requires a signed integer literal divisor".into()
                                );
                            }
                        },
                        _ => {
                            return Err(
                                "remainder requires a signed integer literal divisor".into()
                            );
                        }
                    };
                    if divisor == 0 {
                        return Err("remainder divisor cannot be zero".into());
                    }
                    Expr::Rem(Box::new(lhs), divisor)
                }
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
        self.expect(Token::Word("fnc"))?;
        self.expect(Token::Word("main"))?;
        self.expect(Token::Punct(b'('))?;
        if !self.take(Token::Punct(b')'))? {
            loop {
                let type_first = self.take(Token::Word("int"))?;
                let name = self.name()?;
                if matches!(
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
                ) || self.parameters.contains(&name)
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

fn has_actor_pubsub_surface(text: &str) -> bool {
    // Stage-0 stays scalar-only. Detect the pub/sub surface before ordinary
    // parsing so it cannot accidentally reach CLIF emission as the frontend
    // grows. Comments and quoted literals are inert.
    #[derive(Clone, Copy)]
    enum Tok<'a> {
        Id(&'a str),
        Sym(u8),
    }

    fn is_ident_start(byte: u8) -> bool {
        byte.is_ascii_alphabetic() || byte == b'_'
    }
    fn is_ident_continue(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_'
    }

    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut previous: Option<Tok<'_>> = None;
    let mut before_previous: Option<Tok<'_>> = None;

    while index < bytes.len() {
        let byte = bytes[index];

        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }

        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            index += 2;
            let mut depth = 1usize;
            while index < bytes.len() && depth != 0 {
                if index + 1 < bytes.len()
                    && bytes[index] == b'/'
                    && bytes[index + 1] == b'*'
                {
                    depth += 1;
                    index += 2;
                } else if index + 1 < bytes.len()
                    && bytes[index] == b'*'
                    && bytes[index + 1] == b'/'
                {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            continue;
        }

        if matches!(byte, b'"' | b'\'' | b'`') {
            let quote = byte;
            index += 1;
            let mut escaped = false;
            while index < bytes.len() {
                let current = bytes[index];
                index += 1;
                if escaped {
                    escaped = false;
                } else if current == b'\\' {
                    escaped = true;
                } else if current == quote {
                    break;
                }
            }
            continue;
        }

        let current = if is_ident_start(byte) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_ident_continue(bytes[index]) {
                index += 1;
            }
            Tok::Id(&text[start..index])
        } else {
            index += 1;
            Tok::Sym(byte)
        };

        let found = match (before_previous, previous, current) {
            (Some(Tok::Id("actor")), Some(Tok::Sym(b'.')), Tok::Id("group")) => true,
            (_, Some(Tok::Id("Topic" | "Subscription")), Tok::Sym(b'<')) => true,
            (Some(Tok::Sym(b'.')), Some(Tok::Id("topic")), Tok::Sym(b'<')) => true,
            _ => false,
        };
        if found {
            return true;
        }

        before_previous = previous;
        previous = Some(current);
    }

    false
}

pub fn parse(text: &str) -> Result<Program, String> {
    if has_actor_pubsub_surface(text) {
        return Err("native ActorGroup/Topic source lowering unsupported".into());
    }
    Parser::new(text)?.program()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(e: &Expr, args: &[i64]) -> Result<i64, &'static str> {
        match e {
            Expr::Literal(x) => Ok(*x),
            Expr::Param(i) => Ok(args[*i]),
            Expr::Neg(x) => eval(x, args)?.checked_neg().ok_or("overflow"),
            Expr::Add(a, b) => eval(a, args)?.checked_add(eval(b, args)?).ok_or("overflow"),
            Expr::Sub(a, b) => eval(a, args)?.checked_sub(eval(b, args)?).ok_or("overflow"),
            Expr::Mul(a, b) => eval(a, args)?.checked_mul(eval(b, args)?).ok_or("overflow"),
            Expr::BitXor(a, b) => Ok(eval(a, args)? ^ eval(b, args)?),
            Expr::Rem(lhs, divisor) => {
                let numerator = eval(lhs, args)?;
                if *divisor == -1 {
                    Ok(0) // Java signed remainder: even i64::MIN % -1 is zero.
                } else {
                    numerator.checked_rem(*divisor).ok_or("overflow")
                }
            }
        }
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
    fn signed_literal_remainder() {
        for (source, expected) in [
            ("fnc main():int{return -7%3;}", -1),
            ("fnc main():int{return 7%-3;}", 1),
            ("fnc main():int{return -7%-3;}", -1),
            ("fnc main():int{return 8%3*2;}", 4),
            ("fnc main():int{return 20%6%3;}", 2),
            ("fnc main():int{return -9223372036854775808%-1;}", 0),
        ] {
            let p = parse(source).unwrap();
            assert_eq!(eval(&p.result, &[]), Ok(expected));
        }
        for source in [
            "fnc main(x:int):int{return x%0;}",
            "fnc main(x:int,y:int):int{return x%y;}",
            "fnc main(x:int):int{return x%(1+1);}",
            "fnc main(x:int):int{return x%-x;}",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
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
        ] {
            assert!(parse(source).is_err(), "{source:?}");
        }
        assert!(parse(&format!("fnc main():int{{return {};}}", "(".repeat(150))).is_err());
        assert!(parse(&" ".repeat(MAX_SOURCE + 1)).is_err());
        assert!(parse("fnc main():int{return 42;} // café").is_ok());
    }
}
