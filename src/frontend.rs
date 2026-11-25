//! Fail-closed, bounded *subset* of Oreslang. Not a general Oreslang parser.
#[path = "quoted_source.rs"]
mod quoted_source;
// Use the same line boundaries as the Java/Rust source lexer and the
// quoted-template scanner. Keep byte offsets stable for all diagnostics.
fn line_break_width(bytes: &[u8], at: usize) -> usize {
    match bytes.get(at).copied() {
        Some(b'\r' | b'\n') => 1,
        Some(0xc2) if bytes.get(at + 1) == Some(&0x85) => 2,
        Some(0xe2)
            if bytes.get(at + 1) == Some(&0x80)
                && matches!(bytes.get(at + 2).copied(), Some(0xa8 | 0xa9)) =>
        {
            3
        }
        _ => 0,
    }
}

fn whitespace_width(bytes: &[u8], at: usize) -> usize {
    match bytes.get(at).copied() {
        Some(b' ' | b'\t' | b'\x0b' | b'\x0c') => 1,
        _ => line_break_width(bytes, at),
    }
}

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
    BitAnd(Box<Expr>, Box<Expr>),
    BitOr(Box<Expr>, Box<Expr>),
    NamedRem(Box<Expr>, Box<Expr>),
    NamedMod(Box<Expr>, Box<Expr>),
    NamedPow(Box<Expr>, Box<Expr>),
    ShiftRight8(Box<Expr>),
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
    Quoted(&'a str),
    Punct(u8),
    End,
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    current: Token<'a>,
    nodes: usize,
    parameters: Vec<String>,
    mutex_scan: bool,
}

fn is_comparison_bang(suffix: &str) -> bool {
    suffix.starts_with("!=")
        || suffix.strip_prefix("!eq").is_some_and(|tail| {
            !tail.chars().next().is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        })
}

// Shared block-comment cursor for the scalar lexer and contextual lookahead.
// The actor preflight already recognizes nested block comments; the main lexer
// must not interpret code inside them, and must fail on unterminated/nested bombs.
fn skip_block_comment(bytes: &[u8], start: usize) -> Result<usize, String> {
    if bytes.get(start..start.saturating_add(2)) != Some(&b"/*"[..]) {
        return Err("expected block comment".into());
    }
    let mut index = start + 2;
    let mut depth = 1usize;
    while index < bytes.len() {
        if bytes.get(index..index.saturating_add(2)) == Some(&b"/*"[..]) {
            depth += 1;
            if depth > 128 {
                return Err(format!("block comment nesting limit at byte {index}"));
            }
            index += 2;
        } else if bytes.get(index..index.saturating_add(2)) == Some(&b"*/"[..]) {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return Ok(index);
            }
        } else {
            index += 1;
        }
    }
    Err(format!("unterminated block comment at byte {start}"))
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Result<Self, String> {
        Self::with_mutex_scan(input, false)
    }

    fn with_mutex_scan(input: &'a str, mutex_scan: bool) -> Result<Self, String> {
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
            mutex_scan,
        };
        p.next()?;
        Ok(p)
    }

    /// True when the next significant byte (after whitespace and line
    /// comments) opens a block. Used for contextual `task`/`nb` blocks.
    fn next_opens_block(&self) -> bool {
        let b = self.input.as_bytes();
        let mut at = self.pos;
        loop {
            while at < b.len() {
                let width = whitespace_width(b, at);
                if width == 0 {
                    break;
                }
                at += width;
            }
            if b.get(at..at.saturating_add(2)) == Some(&b"//"[..]) {
                at += 2;
                while at < b.len() && line_break_width(b, at) == 0 {
                    at += 1;
                }
                continue;
            }
            if b.get(at..at.saturating_add(2)) == Some(&b"/*"[..]) {
                at = match skip_block_comment(b, at) {
                    Ok(end) => end,
                    Err(_) => return false,
                };
                continue;
            }
            return b.get(at) == Some(&b'{');
        }
    }

    fn next(&mut self) -> Result<(), String> {
        let b = self.input.as_bytes();
        loop {
            while self.pos < b.len() {
                let width = whitespace_width(b, self.pos);
                if width == 0 {
                    break;
                }
                self.pos += width;
            }
            if b.get(self.pos..self.pos.saturating_add(2)) == Some(&b"//"[..]) {
                self.pos += 2;
                // CR/LF/NEL/Unicode line separators all terminate // comments.
                // A comment must not swallow the following source declaration.
                while self.pos < b.len() && line_break_width(b, self.pos) == 0 {
                    self.pos += 1;
                }
                continue;
            }
            if b.get(self.pos..self.pos.saturating_add(2)) == Some(&b"/*"[..]) {
                self.pos = skip_block_comment(b, self.pos)?;
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
            // Scalar stage-0 has no compiler-owned library linker.
            if self.current == Token::Word("import") {
                return Err("E_LIBRARY_PROFILE_UNSUPPORTED: atto.core/alloc/std and actor modules require verified import/profile lowering".into());
            }
            // Distinguish contextual bangs from comparisons.
            let suffix = &self.input[self.pos..];
            let comparison = is_comparison_bang(suffix);
            if matches!(
                self.current,
                Token::Word(
                    "copy" | "take" | "borrow" | "share" | "pin" | "drop"
                        | "freeze" | "weak" | "ref" | "deref" | "unref"
                )
            ) && b.get(self.pos) == Some(&b'!') && !comparison
            {
                return Err("OWNERSHIP_BANG_BACKEND_UNSUPPORTED: typed ownership and checked-handle native JIT lowering required".into());
            }
            if self.current == Token::Word("mut") {
                let mut at = self.pos;
                while b.get(at).is_some_and(|c| b" \t\r\n\x0b\x0c".contains(c)) {
                    at += 1;
                }
                if b.get(at..at.saturating_add(7)) == Some(&b"borrow!"[..])
                    && !is_comparison_bang(&self.input[at + 6..])
                {
                    return Err("OWNERSHIP_BANG_BACKEND_UNSUPPORTED: typed ownership and native JIT lowering required".into());
                }
            }
            if self.current == Token::Word("new") && !self.mutex_scan {
                return Err("E_ACTOR_MEMORY_GUEST_UNSUPPORTED: class allocation requires actor-owned heap, typed Ref32 and ownership lowering".into());
            }
            if self.current == Token::Word("unsafe") {
                return Err("unsafe source is not supported".into());
            }
            // Match requires a typed CFG and exhaustive pattern lowering.
            if self.current == Token::Word("match") {
                return Err(format!(
                    "MATCH_DEFAULT_BACKEND_UNSUPPORTED: typed patterns, checked fallback and exhaustiveness require CFG lowering at byte {start}"
                ));
            }
            // Macro declarations require compiler-time AST expansion.
            // Stage-0 must reject before CLIF/JIT, including unused macros.
            if self.current == Token::Word("macro") {
                return Err("CRANELIFT_MACRO_EXPANDER_REQUIRED: define macro requires compiler-only typed AST expansion and AOT semantic re-admission; no runtime/JIT fallback".into());
            }
            // Stage-0 has no actor runtime, supervisory CONTROL channel, or
            // minted ActorRef capability. Reject reserved authority syntax
            // lexically, including in unconsumed trailing source, before CLIF.
            // Comments are already skipped above.
            if matches!(
                self.current,
                Token::Word("proxy")
                    | Token::Word("spawn")
                    | Token::Word("stop")
                    | Token::Word("ActorRef")
                    | Token::Word("ActorId" | "Supervisor" | "SupervisorId" | "SupervisorRef")
            ) {
                return Err(
                    "actor and supervisor capabilities are unsupported in Cranelift stage-0".into(),
                );
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
            // Collection and borrow syntax are not represented by scalar
            // stage-0 MIR. Reserve these names before CLIF/JIT output; a
            // parser syntax error is not a verified semantic borrow denial.
            // This prevents a future generic extension from accidentally
            // interpreting them as plain i64 parameters or functions.
            if matches!(
                self.current,
                Token::Word(
                    "Array"
                        | "FixedArray"
                        | "Vector"
                        | "List"
                        | "FixedList"
                        | "Slice"
                        | "Tuple"
                        | "Map"
                        | "Entry"
                        | "Record"
                        | "Set"
                        | "ReadLoan"
                        | "WriteLoan"
                )
            ) {
                return Err("E_SAFE_COLLECTION_ACCESS_UNSUPPORTED: typed collection, bounds-safe Option and static loan provenance lowering are unavailable in Cranelift stage-0".into());
            }
            // cpu/gpu/quantum are execution-domain words reserved by every
            // backend (Java and LLVM reject them as identifiers). This JIT
            // lowers no execution region or CPU/GPU Task, so refuse them
            // here rather than through an incidental syntax error.
            if let Token::Word(word @ ("cpu" | "gpu" | "quantum")) = self.current {
                return Err(format!(
                    "EXECUTION_DOMAIN_UNSUPPORTED: execution domain '{word}' is reserved and not lowered by the Cranelift backend; no implicit CPU fallback"
                ));
            }
            if matches!(self.current, Token::Word("task" | "nb")) && self.next_opens_block() {
                return Err("EXECUTION_DOMAIN_UNSUPPORTED: execution task requires scheduler-aware Future lowering; no implicit inline execution".into());
            }
        } else if matches!(first, b'\'' | b'"' | b'`') {
            self.pos = quoted_source::scan_quoted(self.input, start)?;
            self.current = Token::Quoted(&self.input[start..self.pos]);
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
        } else if first == b'~' {
            return Err(format!(
                "TILDE_AGGREGATE_BACKEND_UNSUPPORTED: typed Map/Record requires validated aggregate layout and ownership at byte {start}"
            ));
        } else if first == b'=' && b.get(self.pos) == Some(&b'>') {
            return Err("E_UNSUPPORTED_MAP_ENTRY: nominal Entry => requires typed key/value, ownership and guest lowering".into());
        } else if first == b'!' {
            // Bang invocation is not a runtime scalar call.
            return Err("CRANELIFT_MACRO_EXPANDER_REQUIRED: bang invocation requires compile-time macro expansion before AOT/CLIF; no runtime/JIT fallback".into());
        } else if b"()+-*{}[];:,><=.".contains(&first) {
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
            Token::Quoted(_) => Err(format!(
                "STRING_BACKEND_UNSUPPORTED: typed String IR and native template interpolation lowering required at byte {}", self.pos
            )),
            Token::Number(n) => {
                let value = integer_digits(n)?
                    .parse::<i64>()
                    .map_err(|_| "i64 literal out of range")?;
                self.next()?;
                self.node(Expr::Literal(value))
            }
            Token::Word("Integer") => {
                // Recognize only these typed signed-i64 intrinsics. Unknown Integer.* calls
                // must reject before emitting native code.
                self.next()?;
                self.expect(Token::Punct(b'.'))?;
                let operation = self.name()?;
                if !matches!(
                    operation.as_str(),
                    "bitXor" | "bitAnd" | "bitOr" | "rem" | "mod" | "pow"
                ) {
                    return Err(format!("unsupported Integer operation: {operation}"));
                }
                self.expect(Token::Punct(b'('))?;
                let lhs = self.expr(0, depth + 1)?;
                self.expect(Token::Punct(b','))?;
                let rhs = self.expr(0, depth + 1)?;
                self.expect(Token::Punct(b')'))?;
                let lhs = Box::new(lhs);
                let rhs = Box::new(rhs);
                self.node(match operation.as_str() {
                    "bitXor" => Expr::BitXor(lhs, rhs),
                    "bitAnd" => Expr::BitAnd(lhs, rhs),
                    "bitOr" => Expr::BitOr(lhs, rhs),
                    "rem" => Expr::NamedRem(lhs, rhs),
                    "mod" => Expr::NamedMod(lhs, rhs),
                    "pow" => Expr::NamedPow(lhs, rhs),
                    _ => unreachable!(),
                })
            }
            Token::Word("critical") | Token::Word("await") | Token::Word("suspends") => Err(
                "E_CRITICAL_UNSUPPORTED: stage-0 has no critical guard or suspension lowering"
                    .into(),
            ),
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
                // Advance first so a macro bang is denied by the lexer even
                // when its name is not a bound scalar parameter.
                self.next()?;
                let index = self
                    .parameters
                    .iter()
                    .position(|p| p == s)
                    .ok_or_else(|| format!("unbound or unsupported identifier: {s}"))?;
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
            // Checked identity cast on the *proven i64* scalar subset only.
            // The stage-0 frontend has no runtime type tags: bool, string,
            // dynamic, nominal, Option/as? and owner-changing casts must
            // continue to reject before executable code is generated.
            // Bind more tightly than multiplication, without changing the
            // underlying i64 value or using an unchecked reinterpretation.
            if self.current == Token::Word("as") && min_bp <= 5 {
                self.next()?;
                if !self.take(Token::Word("int"))? {
                    return Err("unsupported checked cast target (only int is supported)".into());
                }
                continue;
            }
            let (l_bp, r_bp, op) = match self.current {
                Token::Punct(b'+') => (1, 2, b'+'),
                Token::Punct(b'-') => (1, 2, b'-'),
                Token::Punct(b'*') => (3, 4, b'*'),
                Token::Punct(b'>') => (0, 1, b'>'),
                _ => break,
            };
            if l_bp < min_bp {
                break;
            }
            self.next()?;
            if op == b'>' {
                self.expect(Token::Punct(b'>'))?;
            }
            let rhs = self.expr(r_bp, depth + 1)?;
            lhs = self.node(match op {
                b'+' => Expr::Add(Box::new(lhs), Box::new(rhs)),
                b'-' => Expr::Sub(Box::new(lhs), Box::new(rhs)),
                b'*' => Expr::Mul(Box::new(lhs), Box::new(rhs)),
                b'>' => {
                    if rhs != Expr::Literal(8) {
                        return Err("stage-0 only supports signed >> 8 with literal count".into());
                    }
                    Expr::ShiftRight8(Box::new(lhs))
                }
                _ => unreachable!(),
            })?;
        }
        Ok(lhs)
    }
    // Stage-0 can export only the compiled scalar main. No package linker or
    // nominal descriptors are available, and exports convey no guest authority.
    fn file_export(&mut self) -> Result<(), String> {
        self.expect(Token::Word("export"))?;
        self.expect(Token::Word("pub"))?;
        if self.take(Token::Punct(b'['))? {
            if self.current != Token::Word("pkg") {
                return Err(format!("unexpected token at byte {}", self.pos));
            }
            self.next()?;
            self.expect(Token::Punct(b']'))?;
            return Err(
                "EXPORT_PACKAGE_LINKER_UNSUPPORTED: package-private native entrypoint".into(),
            );
        }
        if self.take(Token::Word("class"))? {
            return Err("EXPORT_KIND_UNSUPPORTED: nominal class descriptors not lowered".into());
        }
        self.take(Token::Word("fnc"))?;
        if self.current != Token::Word("main") {
            return Err(format!("unexpected token at byte {}", self.pos));
        }
        self.next()?;
        self.expect(Token::Punct(b';'))?;
        Ok(())
    }
    fn program(mut self) -> Result<Program, String> {
        // File-scoped namespace is metadata only. Stage-0 still requires one
        // scalar main, and no implicit cross-file or nominal type resolution.
        let has_namespace = self.take(Token::Word("use"))?;
        if has_namespace {
            self.expect(Token::Word("namespace"))?;
            let _ = self.name()?;
            self.expect(Token::Punct(b';'))?;
        }
        // Diagnose a repeated header before the generic "expected fnc"
        // branch consumes the invalid token and loses its exact reason.
        if self.current == Token::Word("use") || self.current == Token::Word("namespace") {
            return Err((if has_namespace {
                "CRANELIFT_NAMESPACE_DUPLICATE: duplicate file-level namespace"
            } else {
                "CRANELIFT_NAMESPACE_HEADER_ORDER: namespace must be declared before functions"
            })
            .into());
        }
        let mut seen_export = false;
        while self.current == Token::Word("export") {
            if seen_export {
                return Err("EXPORT_DUPLICATE: main".into());
            }
            self.file_export()?;
            seen_export = true;
        }
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
            if kind == "singleton" {
                return Err("DECL_KIND_UNSUPPORTED: ORES-SINGLETON-UNSUPPORTED: singleton requires compiler-resolved application-domain identity, once-only immutable publication and guarded unique-owner RW lowering; scalar Cranelift cannot provide it".into());
            }
            return Err(format!(
                "DECL_KIND_UNSUPPORTED: {kind} requires nominal/ownership lowering; Cranelift stage-0 supports only scalar fnc main"
            ));
        }
        if matches!(self.current, Token::Word("pattern") | Token::Word("match")) {
            return Err("native type-only ADT pattern lowering unsupported".into());
        }
        if self.current == Token::Word("virtual") {
            return Err("state containers and actor declarations require typed ownership, actor memory and lifecycle lowering; unsupported in scalar Cranelift".into());
        }
        // Stage-0 only compiles synchronous scalar main. These precise
        // modifier checks reject unsupported concurrency before native JIT
        // emission; an arbitrary parser error must not prove effect parity.
        if self.take(Token::Word("async"))? {
            return Err("E_CALLABLE_MODIFIERS: async fnc removed; use suspends".into());
        }
        if self.take(Token::Word("critical"))? {
            if self.current == Token::Word("suspends") {
                return Err("E_CALLABLE_MODIFIERS: critical suspends fnc is forbidden".into());
            }
            return Err(
                "E_CRITICAL_UNSUPPORTED: stage-0 lacks owned critical guard lowering".into(),
            );
        }
        if self.take(Token::Word("suspends"))? {
            if self.current == Token::Word("critical") {
                return Err("E_CALLABLE_MODIFIERS: suspends critical fnc is forbidden".into());
            }
            return Err(
                "E_SUSPENDS_UNSUPPORTED: stage-0 lacks continuation and effect lowering".into(),
            );
        }
        if self.current == Token::Word("suspend") {
            return Err("E_SUSPENDS_UNSUPPORTED: suspends requires continuation lowering".into());
        }
        // The Cranelift stage-0 JIT has no actor heap, mailbox,
        // supervisor/control lane, nominal superclass layout or trait
        // witness dispatch. Recognize actor prefixes explicitly rather
        // than accidentally accepting them as scalar declarations or
        // returning an unrelated "expected fnc" syntax diagnostic.
        if matches!(
            self.current,
            Token::Word("actor")
                | Token::Word("isoactor")
                | Token::Word("shared")
                | Token::Word("untrusted")
                | Token::Word("serial")
                | Token::Word("http")
                | Token::Word("tcp")
                | Token::Word("websocket")
                | Token::Word("unary")
                | Token::Word("stream")
                | Token::Word("abstract")
                | Token::Word("transform")
                | Token::Word("pipe")
        ) {
            return Err(
                "Cranelift source actor hierarchy, traits and mailbox runtime unsupported".into(),
            );
        }
        // Stage-0 is a scalar JIT, not a WebSocket/LiveView runtime.
        // Never silently reinterpret a typed transport actor declaration
        // as a scalar entrypoint or claim machine-code networking parity.
        if matches!(self.current, Token::Word("liveview" | "ws")) {
            return Err(concat!(
                "WebSocket/HTTP/LiveView actors require verified transport, ",
                "capability, ownership and async lowering; scalar Cranelift ",
                "backend does not support network actor execution"
            )
            .into());
        }
        if self.current == Token::Word("isolated") {
            return Err("actor runtime is unsupported by the Cranelift scalar profile".into());
        }
        if self.current == Token::Word("await") {
            return Err("E_SUSPENDS_UNSUPPORTED: await requires continuation lowering".into());
        }
        self.expect(Token::Word("fnc"))?;
        if matches!(self.current, Token::Word("actor") | Token::Word("isoactor")) {
            return Err(
                "Cranelift source actor hierarchy, traits and mailbox runtime unsupported".into(),
            );
        }

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
                            | "supervisor"
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
        // Scalar entry has one proven result type: signed i64. `infer`
        // therefore infers i64 here; it does not admit nominal types or
        // general type inference. All supported return spellings are equivalent.
        let colon_form = self.take(Token::Punct(b':'))?;
        if !colon_form {
            self.expect(Token::Punct(b'-'))?;
            self.expect(Token::Punct(b'>'))?;
        }
        if !self.take(Token::Word("infer"))? {
            self.expect(Token::Word("int"))?;
        }
        // Allow the explicit return/body separator only for colon-form
        // declarations: `: int -> { ... }` and `: infer -> { ... }`.
        if colon_form && self.take(Token::Punct(b'-'))? {
            self.expect(Token::Punct(b'>'))?;
        }
        self.expect(Token::Punct(b'{'))?;
        if self.current == Token::Word("critical")
            || self.current == Token::Word("try")
            || self.current == Token::Word("await")
        {
            return Err(
                "E_CRITICAL_UNSUPPORTED: no critical admission or suspension in scalar stage-0"
                    .into(),
            );
        }
        self.expect(Token::Word("return"))?;
        let result = self.expr(0, 0)?;
        self.expect(Token::Punct(b';'))?;
        self.expect(Token::Punct(b'}'))?;
        while self.current == Token::Word("export") {
            if seen_export {
                return Err("EXPORT_DUPLICATE: main".into());
            }
            self.file_export()?;
            seen_export = true;
        }
        if matches!(self.current, Token::Word("actor" | "supervisor")) {
            return Err(
                "actor and supervisor capabilities are unsupported in Cranelift stage-0".into(),
            );
        }
        // Scan only already-invalid trailing source, preserving compiler-only
        // macro diagnostics without accepting anything after scalar main.
        if self.current == Token::Word("use") || self.current == Token::Word("namespace") {
            return Err((if has_namespace {
                "CRANELIFT_NAMESPACE_DUPLICATE: duplicate file-level namespace"
            } else {
                "CRANELIFT_NAMESPACE_HEADER_ORDER: namespace must be declared before functions"
            })
            .into());
        }
        if self.current != Token::End {
            let at = self.pos;
            while self.current != Token::End {
                self.next()?;
            }
            return Err(format!("unexpected token at byte {at}"));
        }
        self.expect(Token::End)?;
        // Bound the constructed left-associative AST before recursive CLIF lowering.
        validate_ast_depth(&result)?;
        Ok(Program {
            parameters: self.parameters,
            result,
        })
    }
}

fn has_actor_pubsub_surface(text: &str) -> bool {
    has_unsupported_actor_surface(text, false)
}

fn has_actor_pipe_surface(text: &str) -> bool {
    has_unsupported_actor_surface(text, true)
}

fn has_unsupported_actor_surface(text: &str, detect_pipe: bool) -> bool {
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
    let mut third_previous: Option<Tok<'_>> = None;
    let mut pending_pipe = false;
    let mut last_token_end = 0usize;

    while index < bytes.len() {
        let byte = bytes[index];

        let whitespace = whitespace_width(bytes, index);
        if whitespace != 0 {
            index += whitespace;
            continue;
        }

        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            index += 2;
            while index < bytes.len() && line_break_width(bytes, index) == 0 {
                index += 1;
            }
            continue;
        }

        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            index += 2;
            let mut depth = 1usize;
            while index < bytes.len() && depth != 0 {
                if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'*' {
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
            // Shared nested-template grammar keeps source-only actor/security
            // preflight from treating interpolation string contents as code.
            // Any malformed literal is rejected by the actual parser below.
            index = quoted_source::scan_quoted(text, index).unwrap_or(bytes.len());
            continue;
        }

        let token_start = index;
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
            // Fail closed on planned runtime-wide, capability and group syntax,
            // including forms not covered by the original ActorGroup fixtures.
            (_, Some(Tok::Id("runtime_pubsub")), Tok::Sym(b'.')) => true,
            (
                _,
                Some(Tok::Id("ActorGroup" | "TopicPublisher" | "TopicSubscriber" | "PubSub")),
                Tok::Sym(b'<'),
            ) => true,
            (Some(Tok::Id("actor")), Some(Tok::Sym(b'.')), Tok::Id("group")) => true,
            (_, Some(Tok::Sym(b'.')), Tok::Id("pubsub")) => true,
            // A local/imported Topic handle need not spell Topic<T> or
            // actor.group in the same file. Do not let its publication
            // fall through to scalar CLIF/JIT or produce a misleading
            // unknown-member error. A free fnc try_publish remains legal.
            (_, Some(Tok::Sym(b'.')), Tok::Id("try_publish")) => true,
            (_, Some(Tok::Id("Topic" | "Subscription")), Tok::Sym(b'<')) => true,
            (Some(Tok::Sym(b'.')), Some(Tok::Id("topic")), Tok::Sym(b'<')) => true,
            _ => false,
        };
        if !detect_pipe && found {
            return true;
        }
        if detect_pipe {
            let contiguous_pipe_bang = pending_pipe
                && matches!(previous, Some(Tok::Id("pipe" | "rpipe")))
                && matches!(current, Tok::Sym(b'!'))
                && token_start == last_token_end;
            let static_dot = matches!(
                (before_previous, previous, current),
                (
                    Some(Tok::Id("ActorUtils")),
                    Some(Tok::Sym(b'.')),
                    Tok::Id("pipe" | "rpipe")
                )
            );
            let static_colon = matches!(
                (third_previous, before_previous, previous, current),
                (
                    Some(Tok::Id("ActorUtils")),
                    Some(Tok::Sym(b':')),
                    Some(Tok::Sym(b':')),
                    Tok::Id("pipe" | "rpipe")
                )
            );
            let right_start = match current {
                Tok::Id(word) => !matches!(
                    word,
                    "then" | "fi" | "end" | "else" | "as" | "case" | "over"
                ),
                Tok::Sym(b'(' | b'[') => true,
                _ => false,
            };
            if static_dot || static_colon || (pending_pipe && right_start) {
                return true;
            }
            let left_end = match previous {
                Some(Tok::Id(word)) => !matches!(
                    word,
                    "fnc"
                        | "routine"
                        | "method"
                        | "define"
                        | "pub"
                        | "static"
                        | "val"
                        | "let"
                        | "const"
                        | "final"
                        | "return"
                        | "spawn"
                        | "pipe"
                        | "rpipe"
                        | "as"
                        | "case"
                ),
                Some(Tok::Sym(b')' | b']' | b'}')) => true,
                _ => false,
            };
            pending_pipe =
                (matches!(current, Tok::Id("pipe" | "rpipe")) && left_end) || contiguous_pipe_bang;
        }

        last_token_end = index;
        third_previous = before_previous;
        before_previous = previous;
        previous = Some(current);
    }

    false
}

// The parser recursion budget does not bound a long left-associative chain.
// This bounded, iterative traversal rejects an oversized constructed AST.
fn validate_ast_depth(root: &Expr) -> Result<(), String> {
    let mut pending: Vec<(&Expr, usize)> = vec![(root, 1)];
    while let Some((expr, level)) = pending.pop() {
        if level > MAX_DEPTH {
            return Err("expression depth budget exceeded".into());
        }
        match expr {
            Expr::Literal(_) | Expr::Param(_) => {}
            Expr::Neg(inner) | Expr::ShiftRight8(inner) => {
                pending.push((inner.as_ref(), level + 1));
            }
            Expr::Add(left, right)
            | Expr::Sub(left, right)
            | Expr::Mul(left, right)
            | Expr::BitXor(left, right)
            | Expr::BitAnd(left, right)
            | Expr::BitOr(left, right)
            | Expr::NamedRem(left, right)
            | Expr::NamedMod(left, right)
            | Expr::NamedPow(left, right) => {
                pending.push((left.as_ref(), level + 1));
                pending.push((right.as_ref(), level + 1));
            }
        }
    }
    Ok(())
}

/// Admit source bytes only when visual order and line breaks cannot disagree
/// between editor display and the actual lexer. Check ignored comments too.
fn reject_source_unicode_controls(text: &str) -> Result<(), String> {
    for (offset, ch) in text.char_indices() {
        let cp = ch as u32;
        // Check the original UTF-8 source in byte order. Neither quotes nor
        // comments may conceal control bytes before compiler admission.
        if cp == 0 {
            return Err(format!("source_nul at byte {offset}"));
        }
        if matches!(cp, 0x001b | 0x007f) {
            return Err(format!(
                "source_terminal_control at byte {offset}; source contains terminal control"
            ));
        }
        if matches!(cp, 0x0085 | 0x061c | 0x200e | 0x200f | 0x2028 | 0x2029)
            || (0x202a..=0x202e).contains(&cp)
            || (0x2066..=0x2069).contains(&cp)
        {
            return Err(format!(
                "source contains bidirectional Unicode control; source_unicode_control at byte {offset}: U+{cp:04X}"
            ));
        }
    }
    Ok(())
}

/// Token-aware resource-class constructor admission for the currently
/// scalar-only backend. No Mutex runtime, waiter or guest lease exists here.
/// Source is scanned with the existing bounded lexer so commented-out
/// constructors never create false positives or new permissions.
fn reject_unimplemented_mutex_construction(text: &str) -> Result<(), String> {
    // This scanner only classifies constructors, never admits executable code.
    let mut scan = Parser::with_mutex_scan(text, true)?;
    let mut window = [Token::End; 6];
    loop {
        let token = scan.current;
        if token == Token::End { break; }
        window.rotate_left(1);
        window[5] = token;
        if matches!(&window[1..], [Token::Word("Mutex"), Token::Punct(b'<'),
            Token::Word(_), Token::Punct(b'>'), Token::Punct(b'(')]) {
            if window[0] == Token::Word("new") {
                return Err("E_LOCK_UNSUPPORTED: no compiled MutexLease or scheduler exists for Cranelift".into());
            }
            return Err("E_NEW_REQUIRED: Mutex<T> is a nominal resource class; use new Mutex<int>(value)".into());
        }
        scan.next()?;
    }
    Ok(())
}

pub fn parse(text: &str) -> Result<Program, String> {
    if text.len() > MAX_SOURCE {
        return Err("source exceeds 1 MiB limit".into());
    }
    reject_source_unicode_controls(text)?;
    if has_actor_pubsub_surface(text) {
        return Err("native ActorGroup/Topic source lowering unsupported".into());
    }
    if has_actor_pipe_surface(text) {
        return Err("native actor pipe source lowering unsupported".into());
    }
    if text.contains("Mutex") {
        reject_unimplemented_mutex_construction(text)?;
    }
    // The narrow scalar grammar must not hide unsupported collection syntax
    // behind an earlier local declaration. Candidate text only triggers a
    // bounded token scan; quotes and comments are handled by the lexer.
    if text.contains("Map")
        || text.contains("Entry")
        || text.contains("Record")
        || text.contains("=>")
        || text.contains('[')
    {
        let mut scan = Parser::new(text)?;
        while scan.current != Token::End {
            if scan.current == Token::Punct(b'[') {
                return Err("E_UNSUPPORTED_INDEX_WRITE: indexed Record/Map/Struct access requires checked target typing and guest lowering".into());
            }
            scan.next()?;
        }
    }
    Parser::new(text)?.program()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutex_new_constructor_is_required_but_not_yet_runtime_supported() {
        // Constructor grammar applies to all simple nominal type arguments,
        // not just the currently executable scalar int. Nested type arguments
        // remain outside this narrowly bounded source admission gate.
        for ty in ["int", "bool", "float", "Buffer"] {
            let old = format!("pub fnc main():int {{ val counter = Mutex<{ty}>(0); return 0; }}");
            let canonical = format!("pub fnc main():int {{ val counter = new Mutex<{ty}>(0); return 0; }}");
            assert!(parse(&old).unwrap_err().contains("E_NEW_REQUIRED"), "{ty}");
            assert!(parse(&canonical).unwrap_err().contains("E_LOCK_UNSUPPORTED"), "{ty}");
        }
        assert_eq!(eval(&parse("// Mutex<int>(0) is a comment\nfnc main():int{return 42;}").unwrap().result, &[]), Ok(42));
    }

    #[test]
    fn actor_heap_guest_fails_closed_before_cranelift_jit() {
        let node = r#"define class Node as
    int value
    Node? next
end
define actor HeapGraphProbe as
    receive(int message) {
        let first = new Node(42, none)
        let second = new Node(99, first)
        assert(second.next?.value == 42)
    }
end"#;
        let error = parse(node).unwrap_err();
        assert!(error.contains("DECL_KIND_UNSUPPORTED: class"), "{error}");

        for src in [
            "fnc main():int{return new Node(7);}",
            "fnc main():int{return 42;} new Node(7)",
        ] {
            let error = parse(src).unwrap_err();
            assert!(error.contains("E_ACTOR_MEMORY_GUEST_UNSUPPORTED"), "{src}: {error}");
        }
        for inert in [
            "fnc main():int{return 42;} // new Node(7)",
            "fnc main():int{return 42;} /* new Node(7) */",
            "fnc main():int{return 42;} /* define actor A as end */",
        ] {
            assert!(parse(inert).is_ok(), "inert actor memory comment: {inert}");
        }
    }

    #[test]
    fn nested_block_comments_are_inert_but_bounded_and_well_terminated() {
        let guest = "/* define actor Worker */ fnc main(/* ignored */):int { /* outer /* inner */ rest */ return 42; }";
        let program = parse(guest).expect("nested and mixed-position block comments");
        assert_eq!(eval(&program.result, &[]), Ok(42));
        let mixed = "// leading\n/* one */ fnc main():int{return 40 /* two */ +2;} // trailing";
        assert_eq!(eval(&parse(mixed).unwrap().result, &[]), Ok(42));
        for malformed in [
            "fnc main():int{return 42;} /* unfinished",
            "/* forever /* nested */ fnc main():int{return 42;}",
        ] {
            assert!(
                parse(malformed)
                    .unwrap_err()
                    .contains("unterminated block comment")
            );
        }
        let excessive = format!(
            "{}fnc main():int{{return 42;}}{}",
            "/*".repeat(129),
            "*/".repeat(129)
        );
        assert!(
            parse(&excessive)
                .unwrap_err()
                .contains("block comment nesting limit")
        );
    }

    #[test]
    fn pin_and_drop_bang_are_owned_operations_not_scalar_intrinsics() {
        for operation in ["pin!", "drop!", "copy!!"] {
            for expression in [format!("{operation} value"), format!("{operation}(value)")] {
                let source = format!("fnc main(value:int):int{{return {expression};}}");
                let error =
                    parse(&source).expect_err("unsupported owner instruction entered scalar IR");
                assert!(
                    error.contains("OWNERSHIP_BANG_BACKEND_UNSUPPORTED"),
                    "wrong error for {source}: {error}"
                );
            }
        }
        // The bare words are not keywords. Only the bang operators require
        // typed ownership and a proven native lowering.
        let scalar = parse("fnc main(pin:int,drop:int):int{return pin+drop;} // pin! drop! copy!!")
            .expect("inert comment/bare identifiers must stay available");
        assert_eq!(eval(&scalar.result, &[19, 23]), Ok(42));
    }

    #[test]
    fn native_bitand_bitor_depth_is_checked_like_all_other_expr_variants() {
        for (op, expected) in [("bitAnd", 2_i64), ("bitOr", 47_i64)] {
            let valid = format!("fnc main(a:int):int{{return Integer.{op}(a,42);}}");
            let parsed = parse(&valid).expect("named bitwise scalar must remain admitted");
            assert_eq!(eval(&parsed.result, &[7]), Ok(expected));
            let mut nested = String::from("0");
            for _ in 0..(MAX_DEPTH + 5) {
                nested = format!("Integer.{op}({nested},1)");
            }
            // May be caught either by the parser's own limit or the iterative
            // constructed-AST gate; it must never enter recursive native IR.
            let invalid = format!("fnc main():int{{return {nested};}}");
            assert!(
                parse(&invalid).is_err(),
                "nested {op} expression bypassed AST depth budget"
            );
        }
    }

    #[test]
    fn rejects_terminal_control_in_comments_and_literals_before_lowering() {
        for control in ['\u{001b}', '\u{007f}'] {
            for source in [
                format!("fnc main():int{{/*{control}*/return 42;}}"),
                format!("fnc main():int{{return 42;}}//{control}"),
            ] {
                assert!(parse(&source).unwrap_err().contains("terminal control"));
            }
        }
        assert!(parse("fnc main():int{\r\n\treturn 42;\r\n}").is_ok());
    }
    #[test]
    fn rejects_raw_terminal_controls_in_comments_and_literals() {
        for control in ['\u{001b}', '\u{007f}'] {
            for source in [
                format!("/* hidden{control} */ fnc main():int{{return 42;}}"),
                format!("// hidden{control}\nfnc main():int{{return 42;}}"),
                format!("\"hidden{control}\""),
                format!("/* café 😀 */{control}"),
            ] {
                let at = source.find(control).unwrap();
                let err = parse(&source).expect_err("terminal control admitted");
                assert!(
                    err.contains(&format!("source_terminal_control at byte {at}")),
                    "incorrect source location or category: {err}"
                );
            }
        }
    }

    #[test]
    fn first_offender_is_reported_even_with_unicode_prefix() {
        for (source, expected) in [
            ("\0\u{001b}", "source_nul at byte 0"),
            ("\u{007f}\0", "source_terminal_control at byte 0"),
            (
                "/* café 😀 */\u{001b}",
                "source_terminal_control at byte 16",
            ),
        ] {
            let err = parse(source).expect_err("invalid source admitted");
            assert!(err.starts_with(expected), "expected {expected}, got {err}");
        }
        assert!(parse("// café\nfnc main():int{return 42;}").is_ok());
    }

    #[test]
    fn rejects_deceptive_unicode_controls_before_comment_scanning() {
        let baseline = "fnc main():int{return 42;}";
        for cp in [
            0x0085, 0x061c, 0x200e, 0x200f, 0x2028, 0x2029, 0x202a, 0x202b, 0x202c, 0x202d, 0x202e,
            0x2066, 0x2067, 0x2068, 0x2069,
        ] {
            let control = char::from_u32(cp).unwrap();
            for source in [
                format!("/* hidden{control} */ {baseline}"),
                format!("// hidden{control}\n{baseline}"),
                format!("{baseline} // {control}"),
            ] {
                let offset = source.find(control).unwrap();
                let error = parse(&source).expect_err("forbidden source control admitted");
                assert!(
                    error.contains(&format!("source_unicode_control at byte {offset}")),
                    "wrong diagnostic for U+{cp:04X}: {error}"
                );
            }
        }
        assert!(parse("// café\nfnc main():int{return 42;}").is_ok());
    }

    #[test]
    fn quoted_strings_and_nested_templates_are_atomic_and_fail_closed() {
        let positives = [
            "'single'",
            "\"double\"",
            r#"'opposite "quotes"'"#,
            r#""opposite 'quotes'""#,
            r#"'escaped \' quote'"#,
            r#""escaped \" quote""#,
            r#"'unicode \u0041'"#,
            "'raw\nnewline'",
            r#"'\q'"#,
            r#"'\x0G'"#,
            r#"'\uD83D\uDE00'"#,
            "`raw\nmultiline`",
            r#"`escaped \${not_interpolation}`"#,
            r#"`outer ${call({a: "}", b: 1}, /* { */ `inner ${value}`)} end`"#,
            r#"`outer ${fnc({a: "}"} /* outer /* inner */ outer */)} end`"#,
        ];
        for literal in positives {
            assert_eq!(
                quoted_source::scan_quoted(literal, 0),
                Ok(literal.len()),
                "literal incorrectly scanned: {literal}"
            );
            let code = format!("fnc main():int{{return {literal};}}");
            let reason = parse(&code).expect_err("Cranelift cannot lower String yet");
            assert!(
                reason.contains("STRING_BACKEND_UNSUPPORTED"),
                "unexpected source denial for {literal}: {reason}"
            );
        }
        for (literal, reason) in [
            ("'unclosed", "unterminated string/template"),
            (r#"'\u12X4'"#, "invalid Unicode escape"),
            (r#"'\uD800'"#, "invalid Unicode escape"),
            (r#"'\uDFFF'"#, "invalid Unicode escape"),
            (r#"'\u{1f600}'"#, "invalid Unicode escape"),
            (r#"`invalid \q`"#, "invalid template escape"),
            (r#"`invalid \x41`"#, "invalid template escape"),
            (r#"`invalid \u{1f600}`"#, "invalid Unicode escape"),
            ("`invalid \\\nnewline`", "invalid template escape"),
            (
                r#"`only comments ${/* comment */}`"#,
                "empty template interpolation",
            ),
            (
                r#"`line comments ${// comment
}`"#,
                "empty template interpolation",
            ),
            (r#"`bad ${ }`"#, "empty template interpolation"),
            (
                r#"`bad ${ /* unterminated}`"#,
                "unterminated interpolation comment",
            ),
        ] {
            let error = quoted_source::scan_quoted(literal, 0).unwrap_err();
            assert!(error.contains(reason), "wrong error for {literal}: {error}");
            let code = format!("fnc main():int{{return {literal};}}");
            assert!(
                parse(&code).is_err(),
                "malformed literal accepted: {literal}"
            );
        }
        let many = |n: usize| {
            let literal = format!("`{}`", "${'x'}".repeat(n));
            literal
        };
        assert_eq!(
            quoted_source::scan_quoted(&many(1024), 0),
            Ok(many(1024).len())
        );
        assert!(
            quoted_source::scan_quoted(&many(1025), 0)
                .unwrap_err()
                .contains("template substitution limit")
        );
        let braces = |n: usize| {
            let mut literal = String::from("`${");
            literal.push_str(&"{".repeat(n));
            literal.push('x');
            literal.push_str(&"}".repeat(n));
            literal.push('}');
            literal.push('`');
            literal
        };
        assert_eq!(
            quoted_source::scan_quoted(&braces(255), 0),
            Ok(braces(255).len())
        );
        assert!(
            quoted_source::scan_quoted(&braces(256), 0)
                .unwrap_err()
                .contains("interpolation brace limit")
        );
        let nested = |n: usize| {
            let mut literal = String::from("`");
            literal.push_str(&"${`".repeat(n));
            literal.push('x');
            literal.push_str(&"`}".repeat(n));
            literal.push('`');
            literal
        };
        assert_eq!(
            quoted_source::scan_quoted(&nested(16), 0),
            Ok(nested(16).len())
        );
        assert!(
            quoted_source::scan_quoted(&nested(17), 0)
                .unwrap_err()
                .contains("nesting limit")
        );
        let nested_comments = |count: usize| {
            let mut source = String::from("`${");
            source.push_str(&"/*".repeat(count));
            source.push_str("ignored");
            source.push_str(&"*/".repeat(count));
            source.push_str("x}`");
            source
        };
        assert_eq!(
            quoted_source::scan_quoted(&nested_comments(128), 0),
            Ok(nested_comments(128).len())
        );
        assert!(
            quoted_source::scan_quoted(&nested_comments(129), 0)
                .unwrap_err()
                .contains("comment nesting limit")
        );

        // Next Line is Unicode whitespace: it cannot count as code.
        for body in [
            "\u{0085}",
            "\u{0085}/* comment */",
            "\u{0085}\n",
            "\u{0085}\u{2028}\u{2029}",
        ] {
            let template = format!("`x ${{{body}}}`");
            assert!(
                quoted_source::scan_quoted(&template, 0)
                    .unwrap_err()
                    .contains("empty template interpolation")
            );
        }
        let valid_nel = "`x ${\u{0085}x}`";
        assert_eq!(
            quoted_source::scan_quoted(valid_nel, 0),
            Ok(valid_nel.len())
        );

        // Code-like source inside comments remains non-executable.
        assert!(parse("fnc main():int{/* ` ${actor} */ return 42;}").is_ok());
        assert!(parse("fnc main():int{// 'actor' \nreturn 42;}").is_ok());
    }

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

    fn signed_rem(a: i64, b: i64) -> Result<i64, &'static str> {
        if b == 0 {
            return Err("divisor_zero");
        }
        if a == i64::MIN && b == -1 {
            return Ok(0);
        }
        Ok(a % b)
    }

    fn checked_pow(mut base: i64, exponent: i64) -> Result<i64, &'static str> {
        if exponent < 0 {
            return Err("negative_exponent");
        }
        let mut left = exponent as u64;
        let mut result = 1_i64;
        while left != 0 {
            if left & 1 != 0 {
                result = result.checked_mul(base).ok_or("overflow")?;
            }
            left >>= 1;
            if left != 0 {
                base = base.checked_mul(base).ok_or("overflow")?;
            }
        }
        Ok(result)
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
            Expr::BitAnd(a, b) => Ok(eval(a, args)? & eval(b, args)?),
            Expr::BitOr(a, b) => Ok(eval(a, args)? | eval(b, args)?),
            Expr::NamedRem(a, b) => signed_rem(eval(a, args)?, eval(b, args)?),
            Expr::NamedMod(a, b) => {
                let left = eval(a, args)?;
                let right = eval(b, args)?;
                let r = signed_rem(left, right)?;
                if r >= 0 {
                    Ok(r)
                } else if right > 0 {
                    Ok(r + right)
                } else {
                    Ok(r - right)
                }
            }
            Expr::NamedPow(a, b) => checked_pow(eval(a, args)?, eval(b, args)?),
            Expr::ShiftRight8(x) => Ok(eval(x, args)? >> 8),
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
    fn native_bitwise_and_or_parser_and_reference_edges() {
        for (operation, left, right, expected) in [
            ("bitAnd", "7", "3", 3),
            ("bitOr", "4", "3", 7),
            ("bitAnd", "-1", "-9223372036854775808", i64::MIN),
            ("bitOr", "-9223372036854775808", "1", i64::MIN + 1),
        ] {
            let source = format!("fnc main():int{{return Integer.{operation}({left},{right});}}");
            let program = parse(&source).unwrap();
            assert_eq!(eval(&program.result, &[]), Ok(expected), "{source}");
        }
        for invalid in [
            "Integer.bitand(1,2)",
            "Integer.bitOr(1)",
            "Integer.bitAnd(1,2,3)",
            "Integer.bitOr(1 | 2,3)",
        ] {
            let source = format!("fnc main():int{{return {invalid};}}");
            assert!(parse(&source).is_err(), "{source}");
        }
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
    fn odb1_checked_arithmetic_right_shift_by_eight() {
        let p = parse("fnc main(x:int):int{return x>>8;}").unwrap();
        for (n, expected) in [
            (0_i64, 0_i64),
            (255, 0),
            (256, 1),
            (65535, 255),
            (2147483647, 8388607),
            (-1, -1),
            (-256, -1),
            (i64::MIN, -36028797018963968),
        ] {
            assert_eq!(eval(&p.result, &[n]), Ok(expected));
        }
        let add_before_shift = parse("fnc main(x:int):int{return x+1>>8;}").unwrap();
        assert_eq!(eval(&add_before_shift.result, &[255]), Ok(1));
        assert!(eval(&add_before_shift.result, &[i64::MAX]).is_err());
        let chained = parse("fnc main(x:int):int{return x>>8>>8;}").unwrap();
        assert_eq!(eval(&chained.result, &[65536]), Ok(1));
        for invalid in [
            "fnc main(x:int):int{return x>>7;}",
            "fnc main(x:int,y:int):int{return x>>y;}",
            "fnc main(x:int):int{return x>>8+1;}",
            "fnc main(x:int):int{return x>>-8;}",
            "fnc main(x:int):int{return x>>>8;}",
            "fnc main(x:int):int{return x>8;}",
            "fnc main(x:int):int{return x<<8;}",
            "fnc main(x:int):int{return x/256;}",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn critical_suspend_modifiers_fail_closed_with_owned_diagnostics() {
        for (source, expected) in [
            ("async fnc main():int{return 1;}", "E_CALLABLE_MODIFIERS"),
            (
                "critical suspends fnc main():int{return 1;}",
                "E_CALLABLE_MODIFIERS",
            ),
            (
                "suspends critical fnc main():int{return 1;}",
                "E_CALLABLE_MODIFIERS",
            ),
            (
                "critical fnc main():int{return 1;}",
                "E_CRITICAL_UNSUPPORTED",
            ),
            (
                "suspends fnc main():int{return 1;}",
                "E_SUSPENDS_UNSUPPORTED",
            ),
            (
                "fnc main():int{critical {return 1;}}",
                "E_CRITICAL_UNSUPPORTED",
            ),
            (
                "fnc main():int{try critical {return 1;} else {return 2;}}",
                "E_CRITICAL_UNSUPPORTED",
            ),
            (
                "fnc main():int{return critical {1};}",
                "E_CRITICAL_UNSUPPORTED",
            ),
            ("fnc main():int{return await 1;}", "E_CRITICAL_UNSUPPORTED"),
        ] {
            let error = parse(source).expect_err(source);
            assert!(error.contains(expected), "{source}: {error}");
        }
        // Lexical comments are not language modifiers or critical regions.
        assert!(parse("fnc main():int{return 42;} // critical suspends async").is_ok());
        assert!(parse("fnc main():int{return 42;} // try critical await").is_ok());
    }

    #[test]
    fn fail_closed() {
        for source in [
            "fnc main():int{return 1/0;}",
            "fnc main():int{return 5 ^ 3;}",
            "fnc main():int{return Integer.div(5,3);}",
            "fnc main():int{return Integer.bitXor(5);}",
            "fnc main():int{return Integer.bitXor(5,3,1);}",
            "fnc main(Integer:int):int{return Integer;}",
            "fnc main():float{return 1;}",
            "fnc main():int{return x;}",
            "fnc main():int{return 1;} fnc foo():int{return 2;}",
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
        // Well-formed trailing block comments are inert, like trailing line comments.
        assert!(parse("fnc main():int{return 1;} /*no*/").is_ok());
        assert!(
            parse("fnc main():int{return 42;} // comment\rcritical fnc hidden():int{return 1;}")
                .is_err()
        );
        assert!(parse("fnc main():int{return 42;} // comment\r").is_ok());
    }
    #[test]
    fn critical_suspends_v1_fails_closed_without_runtime_lowering() {
        for (source, expected) in [
            ("async fnc main():int{return 1;}", "E_CALLABLE_MODIFIERS"),
            (
                "suspends fnc main():int{return 1;}",
                "E_SUSPENDS_UNSUPPORTED",
            ),
            (
                "suspend fnc main():int{return 1;}",
                "E_SUSPENDS_UNSUPPORTED",
            ),
            (
                "critical fnc main():int{return 1;}",
                "E_CRITICAL_UNSUPPORTED",
            ),
            (
                "suspends critical fnc main():int{return 1;}",
                "E_CALLABLE_MODIFIERS",
            ),
            (
                "fnc main():int{critical{return 1;}}",
                "E_CRITICAL_UNSUPPORTED",
            ),
            (
                "fnc main():int{try critical{return 1;}}",
                "E_CRITICAL_UNSUPPORTED",
            ),
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains(expected), "{source}: {error}");
        }
        assert!(parse("// critical suspends async\nfnc main():int{return 7;}").is_ok());
        let scalar = parse("fnc main(critical_count:int):int{return critical_count+1;}").unwrap();
        assert_eq!(eval(&scalar.result, &[41]), Ok(42));
        let scalar = parse("fnc main(suspend_count:int):int{return suspend_count+1;}").unwrap();
        assert_eq!(eval(&scalar.result, &[41]), Ok(42));
    }
    #[test]
    fn scalar_i64_identity_cast_is_not_inference_or_conversion() {
        // Explicit int casts preserve the already checked scalar i64 value.
        // No untyped local declarations or bool casts are implemented.
        for (source, expected) in [
            ("fnc main():int{return 7 as int + 35;}", 42),
            ("fnc main():int{return (2+5) as int * 6;}", 42),
            ("fnc main():int{return 7 as int as int + 35;}", 42),
            ("fnc main():int{return (7 as int)+35;}", 42),
            ("fnc main(x:int):int{return x as int + 35;}", 42),
            (
                "fnc main():int{return -9223372036854775808 as int;}",
                i64::MIN,
            ),
            (
                "fnc main():int{return (9223372036854775807 as int) - 9223372036854775765;}",
                42,
            ),
        ] {
            let parsed = parse(source).expect(source);
            let args = if parsed.parameters.is_empty() {
                vec![]
            } else {
                vec![7]
            };
            assert_eq!(eval(&parsed.result, &args), Ok(expected), "{source}");
        }
        for (source, expected_error) in [
            (
                "fnc main():int{return 7 as bool;}",
                "unsupported checked cast target",
            ),
            (
                "fnc main():int{return 7 as string;}",
                "unsupported checked cast target",
            ),
            (
                "fnc main():int{return 7 as Object;}",
                "unsupported checked cast target",
            ),
            (
                "fnc main():int{return 7 as int as bool;}",
                "unsupported checked cast target",
            ),
        ] {
            let error = parse(source).unwrap_err();
            assert!(error.contains(expected_error), "{source}: {error}");
        }
        for source in [
            "fnc main():int{return 7 as? int;}",
            "fnc main():int{return true as int;}",
            "fnc main():int{return 7 as int as? int;}",
            "fnc main():int{return 9223372036854775808 as int;}",
            "fnc main():int{return 7 as int + 9223372036854775808;}",
            "fnc main():int{return 7 as int;}\0",
            "fnc main():int{return 7 as int; unsafe}",
            "fnc main(as:int):int{return as;}",
            "fnc main():int{return 7 as int",
        ] {
            assert!(parse(source).is_err(), "{source:?}");
        }
    }
    #[test]
    fn named_arithmetic_refuses_unsafe_values_without_panicking() {
        for (expression, expected) in [
            ("Integer.rem(-9223372036854775808,-1)", Ok(0)),
            ("Integer.mod(-1,-9223372036854775808)", Ok(i64::MAX)),
            ("Integer.pow(-2,63)", Ok(i64::MIN)),
            ("Integer.pow(-1,9223372036854775807)", Ok(-1)),
            ("Integer.pow(2,63)", Err("overflow")),
            ("Integer.pow(3,-1)", Err("negative_exponent")),
            ("Integer.rem(7,0)", Err("divisor_zero")),
            ("Integer.mod(-7,0)", Err("divisor_zero")),
        ] {
            let source = format!("fnc main():int{{return {expression};}}");
            let program = parse(&source).unwrap();
            assert_eq!(eval(&program.result, &[]), expected, "{source}");
        }
    }
    #[test]
    fn actor_pubsub_alias_try_publish_denied_before_scalar_clif() {
        // Identical Git blob across LLVM/Wasm/.NET/JS/HDL/Forge/central:
        // c562ad380bb283da9ffa26160653c1186ab63462.
        // This target is scalar-only; it must not pretend to implement
        // Result<PublishReceipt,PublishError> or checked share! lowering.
        let canonical =
            include_str!("../tests/fixtures/atto-ensemble-v1/canonical-unlowered-try-publish.atto");
        for source in [
            canonical,
            "val receipt=imported_topic.try_publish(share! event)?;",
            "val receipt=local /* outer /* nested */ end */ . try_publish(share! event)?;",
            "val receipt=local?.try_publish(rt share event);",
            "val method=topic.try_publish;",
            "val receipt=topic.\ntry_publish(event);",
            "val receipt=topic.// inert .try_publish\ntry_publish(event);",
        ] {
            assert!(
                has_actor_pubsub_surface(source),
                "unlowered aliased Topic member was accepted by source gate: {source}"
            );
            assert_eq!(
                parse(source).unwrap_err(),
                "native ActorGroup/Topic source lowering unsupported",
                "no scalar CLIF, JIT, or artifact should be produced"
            );
        }
    }

    #[test]
    fn actor_pubsub_alias_preflight_ignores_inert_tokens() {
        for inert in [
            "fnc try_publish(value:int):int{return value;}",
            "fnc main():int{return 42;} // topic.try_publish(share! event)?",
            "fnc main():int{return 42;} /* nested /* x.try_publish() */ tail */",
            r#"fnc main():int{return 42;} "topic.try_publish(share! event)?""#,
            "fnc main(topic:int):int{return topic;}",
            "fnc main():int{return 42;} /* .try_publish member reference */",
            "fnc main():int{return 42;} /* topic. */ try_publish",
            "fnc main():int{return 42;} // imported_topic.try_publish\n",
        ] {
            assert!(
                !has_actor_pubsub_surface(inert),
                "inert syntax must not forge typed pub/sub capability: {inert}"
            );
        }
        let scalar = parse("fnc main():int{return 42;}").unwrap();
        assert_eq!(eval(&scalar.result, &[]), Ok(42));
    }

    #[test]
    fn actor_pubsub_preflight_ignores_inert_text_and_accepts_spacing() {
        assert!(!has_actor_pubsub_surface(
            "fnc main():int{return 42;} // actor.group(x) Topic<String>"
        ));
        assert!(!has_actor_pubsub_surface(
            "fnc main():int{return 42;} /* actor . group (x) .topic<String> */"
        ));
        assert!(!has_actor_pubsub_surface(
            r#"fnc main():int{return 42;} "actor.group(x) Topic<String>""#
        ));
        assert!(has_actor_pubsub_surface("actor . group ( x )"));
        assert!(has_actor_pubsub_surface("value . topic < String > (x)"));
        assert!(has_actor_pubsub_surface("Topic < String >"));
        assert!(has_actor_pubsub_surface("Subscription < String >"));
        assert!(has_actor_pubsub_surface(
            "runtime_pubsub.define_topic<PriceTick>(#prices)"
        ));
        assert!(has_actor_pubsub_surface("TopicPublisher < PriceTick >"));
        assert!(has_actor_pubsub_surface("TopicSubscriber < PriceTick >"));
        assert!(has_actor_pubsub_surface("my_group . pubsub ( )"));
        assert!(!has_actor_pubsub_surface(
            "fnc main():int{return 42;} /* TopicPublisher runtime_pubsub */"
        ));
        assert!(!has_actor_pubsub_surface(
            "fnc main():int{return 42;} // TopicSubscriber\\n"
        ));
        assert!(!has_actor_pubsub_surface(
            "fnc main(TopicPublisher:int):int{return TopicPublisher;}"
        ));
        assert!(!has_actor_pubsub_surface(
            "fnc main(runtime_pubsub:int):int{return runtime_pubsub;}"
        ));
        assert!(parse("fnc main(TopicPublisher:int):int{return TopicPublisher;}").is_ok());
        assert!(parse("fnc main(runtime_pubsub:int):int{return runtime_pubsub;}").is_ok());
    }

    #[test]
    fn actor_pipe_preflight_rejects_source_without_misreading_functional_forward() {
        for src in [
            "return spawn Source() pipe spawn Sink();",
            "return ActorUtils::pipe(spawn Source(), spawn Sink());",
            "return ActorUtils.pipe(spawn Source(), spawn Sink());",
            "return first pipe second;",
            "return first rpipe second;",
            "return sink rpipe source;",
            "return spawn Producer() pipe! spawn Consumer();",
            "return first pipe! second;",
            "return sink rpipe! source;",
            "return consumer rpipe! producer;",
            "return ActorUtils.rpipe(source, sink);",
            "return ActorUtils::rpipe(source, sink);",
        ] {
            assert!(has_actor_pipe_surface(src), "must reject actor pipe: {src}");
            assert!(
                parse(src)
                    .unwrap_err()
                    .contains("native actor pipe source lowering unsupported")
            );
        }
        for inert in [
            "fnc pipe(x:int):int{return x;}",
            "fnc main(pipe:int):int{return pipe+1;}",
            "fnc main(rpipe:int):int{return rpipe+1;}",
            "fnc main():int{return 42;} // sink rpipe source",
            "fnc main():int{return 42;} // first pipe! second",
            "fnc main():int{return 42;} /* sink rpipe! source */",
            r#"fnc main():int{return 42;} "first pipe! second""#,
            "return first pipe ! second;",
            "return sink rpipe /* gap */ ! source;",
            "fnc main():int{return 42;} // spawn A() pipe spawn B()",
            r#"fnc main():int{return 42;} "ActorUtils::pipe(A,B)""#,
            "fnc main(x:int):int{return x |> add_one;}",
        ] {
            assert!(!has_actor_pipe_surface(inert), "must ignore inert: {inert}");
        }
        assert!(
            parse("fnc main(pipe:int):int{return pipe+1;}").is_ok(),
            "a contextual pipe operator must not ban scalar parameters named pipe"
        );
    }

    #[test]
    fn unicode_line_boundaries_align_scalar_lexer_and_actor_preflight() {
        for separator in ["\r", "\n"] {
            let scalar = format!("// inert{separator}fnc main():int{{return 42;}}");
            let p = parse(&scalar).expect("line comment must end at separator");
            assert_eq!(eval(&p.result, &[]), Ok(42), "{separator:?}");
            let inline = format!("fnc{separator}main():int{{return 42;}}");
            assert!(parse(&inline).is_ok(), "whitespace {separator:?}");

            let pubsub = format!("// inert{separator}runtime_pubsub.publish");
            assert!(
                has_actor_pubsub_surface(&pubsub),
                "pubsub hidden after {separator:?}"
            );
            assert!(
                parse(&pubsub)
                    .unwrap_err()
                    .contains("native ActorGroup/Topic"),
                "actor preflight denied with wrong class after {separator:?}"
            );
            let inert = format!("// runtime_pubsub.publish{separator}fnc main():int{{return 42;}}");
            assert!(!has_actor_pubsub_surface(&inert), "comment was active");
            assert!(parse(&inert).is_ok(), "comment was not inert");

            let pipe = format!("// inert{separator}first pipe second");
            assert!(
                has_actor_pipe_surface(&pipe),
                "actor pipe hidden after {separator:?}"
            );

            let task = format!("fnc main():int{{return task // inert{separator}{{ 42 }};}}");
            assert!(
                parse(&task)
                    .unwrap_err()
                    .contains("EXECUTION_DOMAIN_UNSUPPORTED"),
                "task block was not recognized after {separator:?}"
            );
        }
    }
    #[test]
    fn unicode_security_preflight_rejects_control_separators_even_in_literals() {
        for control in ["\u{0085}", "\u{2028}", "\u{2029}"] {
            for source in [
                format!("// comment{control}\nfnc main():int{{return 42;}}"),
                format!("fnc main():int{{return \"quoted{control}\";}}"),
            ] {
                assert!(
                    parse(&source)
                        .unwrap_err()
                        .contains("source_unicode_control"),
                    "Unicode security control must fail before quote/comment admission"
                );
            }
        }
    }

    #[test]
    fn actor_preflight_cr_comments_end_at_carriage_return() {
        assert!(has_actor_pubsub_surface("// inert\rruntime_pubsub.publish"));
        assert!(has_actor_pipe_surface("// inert\rfirst pipe second"));
        assert!(!has_actor_pubsub_surface(
            "// runtime_pubsub.publish\rfnc main():int{return 1;}"
        ));
    }
}
