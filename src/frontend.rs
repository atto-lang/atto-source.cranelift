//! Fail-closed, bounded *subset* of Oreslang. Not a general Oreslang parser.
#[path = "closed_option.rs"]
mod closed_option;
#[path = "quantity_scalar.rs"]
mod quantity_scalar;
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
const MAX_LOCALS: usize = 64;

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
    Local(usize),
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Compare(u8, Box<Expr>, Box<Expr>),
    Choose(Box<Expr>, Box<Expr>, Box<Expr>),
    RefinementCast(Box<Expr>, i64, i64),
    I32Checked(Box<Expr>),
    BitXor(Box<Expr>, Box<Expr>),
    BitAnd(Box<Expr>, Box<Expr>),
    BitOr(Box<Expr>, Box<Expr>),
    NamedRem(Box<Expr>, Box<Expr>),
    NamedMod(Box<Expr>, Box<Expr>),
    NamedPow(Box<Expr>, Box<Expr>),
    ShiftRight8(Box<Expr>),
    /// Evaluate checked scalar expressions in order and keep the final value.
    Seq(Vec<Expr>),
    InlineTask(Box<Expr>),
    Sequence(Vec<Expr>, Box<Expr>),
    /// Eager arguments in the caller; callee locals and result in an isolated lexical slot frame.
    ScopedCall(Vec<Expr>, Vec<Expr>, Box<Expr>),
    ScalarLoop(
        Vec<Expr>,
        Vec<usize>,
        Box<Expr>,
        Vec<LoopStep>,
        Vec<Expr>,
        Box<Expr>,
    ),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopStep {
    Bind(Expr, bool),
    Set(usize, Expr),
    Loop(Expr, Vec<LoopStep>),
    Exit(Expr, bool),
}

#[derive(Debug)]
pub struct Program {
    pub parameters: Vec<String>,
    pub locals: Vec<Expr>,
    pub result: Expr,
    pub trace_steps: Vec<TraceStep>,
}

#[derive(Debug)]
pub enum TraceStep {
    Begin(String),
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token<'a> {
    Word(&'a str),
    Number(&'a str),
    Quoted(&'a str),
    Punct(u8),
    End,
}

// Bounded scale arithmetic: 256-bit normalized components, 512-bit
// intermediates. No new dependency and no host integer overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScaleUInt([u32; 16]);
impl ScaleUInt {
    const ZERO: Self = Self([0; 16]);
    const ONE: Self = Self([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    fn from_u64(n: u64) -> Self {
        let mut x = Self::ZERO;
        x.0[0] = n as u32;
        x.0[1] = (n >> 32) as u32;
        x
    }
    fn low64(self) -> u64 {
        u64::from(self.0[0]) | (u64::from(self.0[1]) << 32)
    }
    fn fits256(self) -> bool {
        self.0[8..].iter().all(|v| *v == 0)
    }
    fn cmp(self, b: Self) -> std::cmp::Ordering {
        self.0.iter().rev().cmp(b.0.iter().rev())
    }
    fn sub(&mut self, b: Self) {
        let mut borrow = 0u64;
        for i in 0..16 {
            let a = u64::from(self.0[i]);
            let v = u64::from(b.0[i]) + borrow;
            self.0[i] = a.wrapping_sub(v) as u32;
            borrow = u64::from(a < v);
        }
    }
    fn shift(&mut self) {
        let mut carry = 0;
        for word in &mut self.0 {
            let next = *word >> 31;
            *word = (*word << 1) | carry;
            carry = next;
        }
    }
    fn mul256(self, b: Self) -> Self {
        let mut out = Self::ZERO;
        for i in 0..8 {
            let mut carry = 0u64;
            for j in 0..8 {
                let v = u64::from(self.0[i]) * u64::from(b.0[j]) + u64::from(out.0[i + j]) + carry;
                out.0[i + j] = v as u32;
                carry = v >> 32;
            }
            out.0[i + 8] = carry as u32;
        }
        out
    }
    fn divmod(self, b: Self) -> (Self, Self) {
        debug_assert!(b != Self::ZERO);
        let mut q = Self::ZERO;
        let mut r = Self::ZERO;
        for i in (0..512).rev() {
            let carry = r.0[15] >> 31 != 0;
            r.shift();
            r.0[0] |= (self.0[i / 32] >> (i % 32)) & 1;
            if carry || !r.cmp(b).is_lt() {
                r.sub(b);
                q.0[i / 32] |= 1u32 << (i % 32);
            }
        }
        (q, r)
    }
    fn gcd(mut a: Self, mut b: Self) -> Self {
        while b != Self::ZERO {
            let r = a.divmod(b).1;
            a = b;
            b = r;
        }
        a
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuantityScale(ScaleUInt, ScaleUInt);
impl QuantityScale {
    const ONE: Self = Self(ScaleUInt::ONE, ScaleUInt::ONE);
    fn inv(self) -> Self {
        Self(self.1, self.0)
    }
    fn mul(self, rhs: Self) -> Result<Self, String> {
        let n = self.0.mul256(rhs.0);
        let d = self.1.mul256(rhs.1);
        let g = ScaleUInt::gcd(n, d);
        let n = n.divmod(g).0;
        let d = d.divmod(g).0;
        if !n.fits256() || !d.fits256() {
            return Err("E_UNIT_LIMIT: scale components exceed 256 bits".into());
        }
        Ok(Self(n, d))
    }
    fn convert(self, v: i64, target: Self) -> Result<i64, String> {
        if v == 0 {
            return Ok(0);
        }
        let n = self.0.mul256(target.1);
        let d = self.1.mul256(target.0);
        let g = ScaleUInt::gcd(n, d);
        let n = n.divmod(g).0;
        let d = d.divmod(g).0;
        let (q, r) = ScaleUInt::from_u64(v.unsigned_abs()).divmod(d);
        if r != ScaleUInt::ZERO {
            return Err("E_UNIT_INEXACT: integer conversion".into());
        }
        let limit = if v < 0 { 1u64 << 63 } else { i64::MAX as u64 };
        if n.cmp(ScaleUInt::from_u64(limit)).is_gt()
            || q.cmp(ScaleUInt::from_u64(limit / n.low64())).is_gt()
        {
            return Err("E_UNIT_RANGE: integer conversion".into());
        }
        let m = q.low64() * n.low64();
        if v < 0 && m == 1u64 << 63 {
            return Ok(i64::MIN);
        }
        Ok(if v < 0 { -(m as i64) } else { m as i64 })
    }
}
struct Parser<'a> {
    input: &'a str,
    pos: usize,
    newline_before_current: bool,
    current: Token<'a>,
    nodes: usize,
    parameters: Vec<String>,
    mutex_scan: bool,
    strict_fixed_arrays: bool,
    closed_option_profile: bool,
    bindings: Vec<(String, Option<i64>, usize)>,
    local_initializers: Vec<Expr>,
    pair_scope_checked: bool,
    entry_scope_checked: bool,
    unsigned_scalar: bool,
}

fn is_comparison_bang(suffix: &str) -> bool {
    suffix.starts_with("!=")
        || suffix.strip_prefix("!eq").is_some_and(|tail| {
            !tail
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        })
}

// Shared block-comment cursor for the scalar lexer and contextual lookahead.
// The actor preflight already recognizes nested block comments; the main lexer
// must not interpret code inside them, and must fail on unterminated/nested bombs.
pub(crate) fn skip_block_comment(bytes: &[u8], start: usize) -> Result<usize, String> {
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
            newline_before_current: false,
            current: Token::End,
            nodes: 0,
            parameters: Vec::new(),
            mutex_scan,
            strict_fixed_arrays: false,
            closed_option_profile: false,
            bindings: Vec::new(),
            local_initializers: Vec::new(),
            pair_scope_checked: false,
            entry_scope_checked: false,
            unsigned_scalar: false,
        };
        p.next()?;
        Ok(p)
    }

    /// True when the next significant byte (after whitespace and line
    /// comments) opens a block. Used for contextual `task`/`nb` blocks.
    fn next_significant_is(&self, expected: u8) -> bool {
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
            return b.get(at) == Some(&expected);
        }
    }
    fn next_opens_block(&self) -> bool {
        self.next_significant_is(b'{')
    }

    fn next_significant_word_is(&self, expected: &str) -> bool {
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
            let word = expected.as_bytes();
            return b.get(at..at.saturating_add(word.len())) == Some(word)
                && b.get(at + word.len())
                    .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_');
        }
    }

    fn next(&mut self) -> Result<(), String> {
        let previous_end = self.pos;
        let b = self.input.as_bytes();
        let preceded_by_sync = self.current == Token::Word("sync");
        let preceded_by_as = self.current == Token::Word("as");
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
        self.newline_before_current =
            (previous_end..self.pos).any(|offset| line_break_width(b, offset) != 0);
        if self.pos == b.len() {
            self.current = Token::End;
            return Ok(());
        }
        let start = self.pos;
        // Exact UTF-8 suspension aliases share the existing fail-closed effect gate.
        if self.input[start..].starts_with('⧖') || self.input[start..].starts_with('∂') {
            self.pos += 3;
            self.current = Token::Word("suspends");
            return Ok(());
        }
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
            if matches!(name, "requires" | "ensures") {
                self.current = Token::Punct(b'@');
                return Ok(());
            }
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
            let spelling = &self.input[start..self.pos];
            self.current = Token::Word(if spelling == "susp" {
                "suspends"
            } else {
                spelling
            });
            // Scalar stage-0 has no compiler-owned library linker.
            if self.current == Token::Word("import") {
                return Err("E_LIBRARY_PROFILE_UNSUPPORTED: atto.core/alloc/std and actor modules require verified import/profile lowering".into());
            }
            // Distinguish contextual bangs from comparisons.
            let suffix = &self.input[self.pos..];
            let comparison = is_comparison_bang(suffix);
            // Enforce the complete contextual suffix before semantic denial.
            if matches!(
                self.current,
                Token::Word(
                    "trap"
                        | "throw"
                        | "raise"
                        | "panic"
                        | "copy"
                        | "take"
                        | "move"
                        | "borrow"
                        | "share"
                        | "pin"
                        | "drop"
                        | "pipe"
                        | "rpipe"
                        | "stop"
                        | "expand"
                        | "freeze"
                        | "weak"
                        | "ref"
                        | "deref"
                        | "unref"
                )
            ) && b.get(self.pos) == Some(&b'!')
                && !comparison
            {
                let after = self.pos
                    + if self.current == Token::Word("copy") && b.get(self.pos + 1) == Some(&b'!') {
                        2
                    } else {
                        1
                    };
                if !b.get(after).is_some_and(|c| b" \t\r\n\x0b\x0c".contains(c)) {
                    return Err(
                        "E_BANG_KEYWORD_WHITESPACE: whitespace required after bang keyword".into(),
                    );
                }
            }
            if matches!(
                self.current,
                Token::Word(
                    "copy"
                        | "take"
                        | "move"
                        | "borrow"
                        | "share"
                        | "pin"
                        | "drop"
                        | "freeze"
                        | "weak"
                        | "ref"
                        | "deref"
                        | "unref"
                )
            ) && b.get(self.pos) == Some(&b'!')
                && !comparison
            {
                return Err("OWNERSHIP_BANG_BACKEND_UNSUPPORTED: typed ownership and checked-handle native JIT lowering required".into());
            }
            if self.closed_option_profile
                && self.current == Token::Word("trap")
                && b.get(self.pos) == Some(&b'!')
                && !comparison
            {
                self.pos += 1;
                self.current = Token::Word("trap!");
                return Ok(());
            }
            if matches!(
                self.current,
                Token::Word("trap" | "throw" | "raise" | "panic")
            ) && b.get(self.pos) == Some(&b'!')
                && !comparison
            {
                return Err("EXCEPTION_BANG_BACKEND_UNSUPPORTED: typed guest Error/Result, unwind and actor supervisor lowering required".into());
            }
            if self.current == Token::Word("mut") {
                let mut at = self.pos;
                while b.get(at).is_some_and(|c| b" \t\r\n\x0b\x0c".contains(c)) {
                    at += 1;
                }
                if b.get(at..at.saturating_add(7)) == Some(&b"borrow!"[..])
                    && !is_comparison_bang(&self.input[at + 6..])
                {
                    if !b
                        .get(at + 7)
                        .is_some_and(|c| b" \t\r\n\x0b\x0c".contains(c))
                    {
                        return Err(
                            "E_BANG_KEYWORD_WHITESPACE: whitespace required after bang keyword"
                                .into(),
                        );
                    }
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
            if !self.closed_option_profile && self.current == Token::Word("match") {
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
            if !self.closed_option_profile
                && matches!(
                    self.current,
                    Token::Word("Option")
                        | Token::Word("Result")
                        | Token::Word("Some")
                        | Token::Word("None")
                        | Token::Word("Ok")
                        | Token::Word("Err")
                        | Token::Word("trap")
                )
            {
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
            ) && !(self.strict_fixed_arrays && self.current == Token::Word("Array"))
                && !(self.current == Token::Word("Entry") && self.next_significant_is(b'.'))
            {
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
            if (self.current == Token::Word("nb") && self.next_opens_block())
                || (self.current == Token::Word("task")
                    && (self.next_opens_block() || self.next_significant_is(b'<'))
                    && !preceded_by_sync)
            {
                return Err("EXECUTION_DOMAIN_UNSUPPORTED: E_TASK_UNSUPPORTED: execution task requires scheduler-aware Future lowering; no implicit inline execution".into());
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
        } else if first == b'?' && preceded_by_as {
            return Err(
                "E_REFINEMENT_CAST: optional checked casts are not admitted in this profile".into(),
            );
        } else if first == b'!' && b.get(self.pos) != Some(&b'=') {
            // Bang invocation is not a runtime scalar call.
            return Err("CRANELIFT_MACRO_EXPANDER_REQUIRED: bang invocation requires compile-time macro expansion before AOT/CLIF; no runtime/JIT fallback".into());
        } else if (self.closed_option_profile && first == b'?')
            || b"()+-*/{}[];:,><=.!&|".contains(&first)
        {
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
    /// Sequences are scoped, never tuples or callable argument lists.
    /// Even unused earlier values must retain arithmetic trap behavior.
    fn sequence(&mut self, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("expression depth budget exceeded".into());
        }
        if self.current == Token::Punct(b')') {
            return Err("E_SEQ_EMPTY: sequence must have at least one expression".into());
        }
        let mut expressions = Vec::new();
        loop {
            if expressions.len() >= 16 {
                return Err("E_SEQ_LIMIT: at most 16 scalar expressions per sequence".into());
            }
            if matches!(self.current, Token::Punct(b',' | b';' | b')') | Token::End) {
                return Err("E_SEQ_COMMA: missing scalar sequence expression".into());
            }
            expressions.push(self.expr(0, depth + 1)?);
            if !self.take(Token::Punct(b','))? {
                break;
            }
        }
        self.expect(Token::Punct(b')'))?;
        self.node(Expr::Seq(expressions))
    }
    // Deliberately bounded to the trusted, non-suspending scalar subset.
    // Every inline return exits this lexical body, not the enclosing main().
    // Unknown-effect calls, child spawns, loans, cleanup and suspension reject.
    fn inline_task(&mut self, depth: usize, discard: bool) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("expression depth budget exceeded".into());
        }
        self.expect(Token::Word("sync"))?;
        self.expect(Token::Word("task"))?;
        let payload = if self.take(Token::Punct(b'<'))? {
            let is_int = if self.take(Token::Word("int"))? {
                true
            } else if self.take(Token::Word("void"))? {
                false
            } else {
                return Err(
                    "E_TASK_RETURN_TYPE: inline scalar tasks accept int or void only".into(),
                );
            };
            self.expect(Token::Punct(b'>'))?;
            Some(is_int)
        } else {
            None
        };
        // A discarded inline task is a void-only statement: do not erase
        // nonvoid payload types, even if the task body has no return.
        if discard && payload == Some(true) {
            return Err("E_TASK_RETURN_TYPE: do sync task requires void".into());
        }
        self.expect(Token::Punct(b'{'))?;
        let effects = self.inline_statements(depth + 1)?;
        let value = if self.take(Token::Word("return"))? {
            if self.take(Token::Punct(b';'))? {
                None
            } else {
                let value = self.expr(0, depth + 1)?;
                self.expect(Token::Punct(b';'))?;
                Some(value)
            }
        } else {
            None
        };
        self.expect(Token::Punct(b'}'))?;
        if (payload == Some(true) && value.is_none()) || (payload == Some(false) && value.is_some())
        {
            return Err("E_TASK_RETURN_TYPE: inline task payload and return do not agree".into());
        }
        if discard && value.is_some() {
            return Err("E_TASK_RETURN_TYPE: do sync task cannot return a value".into());
        }
        if !discard && value.is_none() {
            return Err("E_TASK_RETURN_TYPE: void inline task cannot be used as int".into());
        }
        // Internal sentinel is unobservable: void tasks occur only in
        // discarded statement context, and their result never escapes.
        let value = match value {
            Some(v) => v,
            None => self.node(Expr::Literal(0))?,
        };
        let body = if effects.is_empty() {
            value
        } else {
            self.node(Expr::Sequence(effects, Box::new(value)))?
        };
        self.node(Expr::InlineTask(Box::new(body)))
    }

    fn inline_statements(&mut self, depth: usize) -> Result<Vec<Expr>, String> {
        let mut effects = Vec::new();
        loop {
            let do_prefix = self.take(Token::Word("do"))?;
            if self.current != Token::Word("sync") {
                if do_prefix {
                    return Err(
                        "E_TASK_UNSUPPORTED: do task requires cancellable child lowering".into(),
                    );
                }
                break;
            }
            if !do_prefix {
                return Err("E_SYNTAX: void sync task statement requires do".into());
            }
            effects.push(self.inline_task(depth + 1, true)?);
            // A semicolon after do sync task is optional in the canonical grammar.
            self.take(Token::Punct(b';'))?;
            if effects.len() > MAX_NODES {
                return Err("AST node budget exceeded".into());
            }
        }
        Ok(effects)
    }

    fn checked_i64_cast_target(&mut self) -> Result<(), String> {
        if !self.take(Token::Word("int"))? && !self.take(Token::Word("i64"))? {
            return Err("unsupported checked cast target (only int/i64 is supported)".into());
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("AST node budget exceeded".into());
        }
        Ok(())
    }

    fn atom(&mut self, depth: usize) -> Result<Expr, String> {
        if self.unsigned_scalar {
            if depth > MAX_DEPTH {
                return Err("expression depth budget exceeded".into());
            }
            if self.take(Token::Punct(b'+'))? {
                return self.atom(depth + 1);
            }
            if self.take(Token::Punct(b'('))? {
                let value = self.expr(0, depth + 1)?;
                self.expect(Token::Punct(b')'))?;
                return Ok(value);
            }
            return match self.current {
                Token::Number(n) => {
                    let value = integer_digits(n)?
                        .parse::<u64>()
                        .map_err(|_| "u64 literal out of range")?;
                    self.next()?;
                    self.node(Expr::Literal(value as i64))
                }
                Token::Word(name) => {
                    let index = self
                        .parameters
                        .iter()
                        .position(|p| p == name)
                        .ok_or("unsupported unsigned operand")?;
                    self.next()?;
                    self.node(Expr::Param(index))
                }
                _ => Err(
                    "unsigned scalar requires a literal, parameter or parenthesized expression"
                        .into(),
                ),
            };
        }
        let value = self.atom_base(depth)?;
        while self.current == Token::Punct(b':')
            && self.input.as_bytes().get(self.pos) == Some(&b':')
        {
            self.expect(Token::Punct(b':'))?;
            self.expect(Token::Punct(b':'))?;
            self.expect(Token::Punct(b'<'))?;
            self.checked_i64_cast_target()?;
            self.expect(Token::Punct(b'>'))?;
        }
        Ok(value)
    }

    // Restricted checked i64 expression projection. No owned Pair ABI is constructed.
    // Scan source tokens once per compilation, not once per projection.
    fn check_pair_scope(&mut self) -> Result<(), String> {
        if self.pair_scope_checked {
            return Ok(());
        }
        let mut scan = Parser::new(self.input)?;
        let mut previous = Token::End;
        while scan.current != Token::End {
            if scan.current == Token::Word("Pair") {
                if previous == Token::Punct(b'.')
                    || previous == Token::Punct(b':')
                    || !scan.next_significant_is(b'.')
                {
                    return Err("E_PAIR_SCOPE_UNSUPPORTED: competing Pair name".into());
                }
                scan.next()?;
                scan.expect(Token::Punct(b'.'))?;
                scan.expect(Token::Word("of"))?;
                previous = Token::Word("of");
                continue;
            }
            previous = scan.current;
            scan.next()?;
        }
        self.pair_scope_checked = true;
        Ok(())
    }

    fn check_entry_scope(&mut self) -> Result<(), String> {
        if self.entry_scope_checked {
            return Ok(());
        }
        let mut scan = Parser::new(self.input)?;
        let mut previous = Token::End;
        while scan.current != Token::End {
            if scan.current == Token::Word("Entry") {
                if previous == Token::Punct(b'.')
                    || previous == Token::Punct(b':')
                    || !scan.next_significant_is(b'.')
                {
                    return Err("E_ENTRY_SCOPE_UNSUPPORTED: competing Entry name".into());
                }
                scan.next()?;
                scan.expect(Token::Punct(b'.'))?;
                scan.expect(Token::Word("of"))?;
                previous = Token::Word("of");
                continue;
            }
            previous = scan.current;
            scan.next()?;
        }
        self.entry_scope_checked = true;
        Ok(())
    }

    // A restricted i64 projection: evaluate both operands in the caller's
    // frame, then select an already evaluated slot in an isolated frame.
    // No first-class Entry allocation or arbitrary-key ABI is admitted.
    fn entry_projection(&mut self, depth: usize) -> Result<Expr, String> {
        self.check_entry_scope()?;
        self.expect(Token::Word("Entry"))?;
        self.expect(Token::Punct(b'.'))?;
        self.expect(Token::Word("of"))?;
        if self.take(Token::Punct(b'<'))? {
            self.expect(Token::Word("int"))?;
            self.expect(Token::Punct(b','))?;
            self.expect(Token::Word("int"))?;
            self.expect(Token::Punct(b'>'))?;
        }
        self.expect(Token::Punct(b'('))?;
        let first = self.expr(0, depth + 1)?;
        self.expect(Token::Punct(b','))?;
        let second = self.expr(0, depth + 1)?;
        // Only proven i64 parameters and arithmetic are admitted. Do not
        // infer types for runtime locals, calls, booleans or arbitrary keys.
        let mut pending = vec![&first, &second];
        while let Some(expr) = pending.pop() {
            match expr {
                Expr::Literal(_) | Expr::Param(_) => {}
                Expr::Neg(inner) => pending.push(inner),
                Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) => {
                    pending.push(a);
                    pending.push(b);
                }
                _ => {
                    return Err("E_ENTRY_VALUE_UNSUPPORTED: checked i64 operands required".into());
                }
            }
        }
        self.expect(Token::Punct(b')'))?;
        if !self.take(Token::Punct(b'.'))? {
            return Err("E_ENTRY_ESCAPE_UNSUPPORTED: native owned Entry ABI unavailable".into());
        }
        let slot = if self.take(Token::Word("key"))? {
            0
        } else if self.take(Token::Word("value"))? {
            1
        } else {
            return Err("E_ENTRY_PROJECTION_UNSUPPORTED: expected key or value".into());
        };
        let body = self.node(Expr::Local(slot))?;
        self.node(Expr::ScopedCall(
            vec![first, second],
            vec![],
            Box::new(body),
        ))
    }

    fn pair_projection(&mut self, depth: usize) -> Result<Expr, String> {
        self.check_pair_scope()?;
        self.expect(Token::Word("Pair"))?;
        self.expect(Token::Punct(b'.'))?;
        self.expect(Token::Word("of"))?;
        if self.take(Token::Punct(b'<'))? {
            if self.take(Token::Word("string"))? {
                self.expect(Token::Punct(b','))?;
                self.expect(Token::Word("int"))?;
            } else {
                self.expect(Token::Word("int"))?;
                if self.current == Token::Punct(b',') {
                    return Err(
                        "E_PAIR_KEY_TYPE: Pair first generic argument must be string".into(),
                    );
                }
            }
            self.expect(Token::Punct(b'>'))?;
        }
        self.expect(Token::Punct(b'('))?;
        let Token::Quoted(raw) = self.current else {
            return Err("E_PAIR_KEY_TYPE: Pair requires a string key".into());
        };
        // The normal quoted-source scanner validates the token. Restrict
        // discarded keys to plain ASCII strings; no templates or escapes.
        let bytes = raw.as_bytes();
        if bytes.len() < 2 || !matches!(bytes[0], b'\'' | b'"') {
            return Err("E_PAIR_KEY_TYPE: Pair requires a string key".into());
        }
        if !bytes[1..bytes.len() - 1]
            .iter()
            .all(|c| (32..=126).contains(c) && *c != 92 && *c != 36)
        {
            return Err("E_PAIR_KEY_UNSUPPORTED: plain literal string required".into());
        }
        self.next()?;
        self.expect(Token::Punct(b','))?;
        let value = self.expr(0, depth + 1)?;
        // Stage-0 has no general type witness for locals or effects. Permit
        // only known i64 parameters, literals and checked arithmetic trees.
        // Iterative inspection is bounded by the parser's existing node cap.
        let mut pending = vec![&value];
        while let Some(expr) = pending.pop() {
            match expr {
                Expr::Literal(_) | Expr::Param(_) => {}
                Expr::Neg(inner) => pending.push(inner),
                Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) => {
                    pending.push(a);
                    pending.push(b);
                }
                _ => {
                    return Err("E_PAIR_VALUE_UNSUPPORTED: checked i64 expression required".into());
                }
            }
        }
        self.expect(Token::Punct(b')'))?;
        if !self.take(Token::Punct(b'.'))? {
            return Err("E_PAIR_ESCAPE_UNSUPPORTED: native owned Pair ABI unavailable".into());
        }
        if !self.take(Token::Word("second"))? {
            return Err(
                "E_PAIR_PROJECTION_UNSUPPORTED: only i64 second projection admitted".into(),
            );
        }
        Ok(value)
    }

    fn atom_base(&mut self, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("expression depth budget exceeded".into());
        }
        if self.take(Token::Punct(b'<'))? {
            self.checked_i64_cast_target()?;
            self.expect(Token::Punct(b'>'))?;
            return self.atom(depth + 1);
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
        if self.take(Token::Word("seq"))? {
            self.expect(Token::Punct(b'('))?;
            return self.sequence(depth + 1);
        }
        if self.take(Token::Punct(b'('))? {
            if self.take(Token::Punct(b'>'))? {
                return self.sequence(depth + 1);
            }
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
            Token::Word("sync") => {
                if !self.next_significant_word_is("task") {
                    return Err("E_RESERVED: sync is reserved for inline task execution".into());
                }
                self.inline_task(depth + 1, false)
            }
            Token::Word("Pair") if self.next_significant_is(b'.') => self.pair_projection(depth),
            Token::Word("Entry") if self.next_significant_is(b'.') => self.entry_projection(depth),
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
                self.next()?;
                let value = if let Some(index) = self.parameters.iter().position(|p| p == s) {
                    Expr::Param(index)
                } else if let Some((_, constant, runtime_slot)) =
                    self.bindings.iter().find(|(name, _, _)| name == s)
                {
                    // Const references are already typed/folded, while val
                    // references retain a runtime SSA slot.
                    match constant {
                        Some(value) => Expr::Literal(*value),
                        None => Expr::Local(*runtime_slot),
                    }
                } else {
                    return Err(format!("unbound or unsupported identifier: {s}"));
                };
                self.node(value)
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
            if !self.unsigned_scalar && self.current == Token::Word("as") && min_bp <= 5 {
                self.next()?;
                self.checked_i64_cast_target()?;
                continue;
            }
            let (l_bp, r_bp, op) = match self.current {
                Token::Punct(b'+') => (1, 2, b'+'),
                Token::Punct(b'-') => (1, 2, b'-'),
                Token::Punct(b'*') => (3, 4, b'*'),
                Token::Punct(b'/') => (3, 4, b'/'),
                Token::Punct(b'>') if !self.unsigned_scalar => (0, 1, b'>'),
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
                b'/' => Expr::Div(Box::new(lhs), Box::new(rhs)),
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
    // Separate bounded quantity profile; every admitted quantity is typed
    // before erasure into the existing checked scalar IR.
    fn program(mut self) -> Result<Program, String> {
        if self.current == Token::Word("type")
            || (self.current == Token::Word("define")
                && (self.next_significant_word_is("dimension")
                    || self.next_significant_word_is("unit")))
        {
            return quantity_scalar::parse_mode(self, false);
        }
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
        let mut export_count = 0usize;
        while self.current == Token::Word("export") {
            if export_count >= 128 {
                return Err("EXPORT_SOURCE_LIMIT: too many directives".into());
            }
            self.file_export()?;
            export_count += 1;
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
                if name == "sync" {
                    return Err("E_RESERVED: sync is not a legal parameter".into());
                }
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
                            | "seq"
                            | "sync"
                            | "yield"
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
        // This is deliberately a bounded, flat scalar subset. Local const
        // and type-first int require constexpr; val may be runtime-dependent.
        // The file/module define-const environment exception is not local.
        loop {
            let kind = match self.current {
                Token::Word("const") => "const",
                Token::Word("int") => "int",
                Token::Word("val") => "val",
                _ => break,
            };
            self.next()?;
            if self.current == Token::Word("mut") {
                // const mut on scalar int is never a writable variable,
                // and val mut requires a typed referent/loan backend.
                return if kind == "val" {
                    Err("E_MUT_BACKEND_UNSUPPORTED: no scalar referent mutability lowering".into())
                } else {
                    Err("E_CONST_MUT_UNSUPPORTED: scalar const mut requires a non-scalar owned referent".into())
                };
            }
            let typed = kind == "int" || self.take(Token::Word("int"))?;
            let name = self.name()?;
            if matches!(name.as_str(), "sync" | "task" | "yield") {
                return Err("E_PIPE_NAME: reserved authority/scheduler binding".into());
            }
            if is_java_v1_reserved_parameter_name(&name)
                || matches!(name.as_str(), "Integer" | "main" | "int" | "seq")
                || self.parameters.contains(&name)
                || self.bindings.iter().any(|(known, _, _)| known == &name)
            {
                return Err("invalid, reserved or duplicate local identifier".into());
            }
            if !typed && self.take(Token::Punct(b':'))? {
                self.expect(Token::Word("int"))?;
            }
            if self.bindings.len() >= MAX_LOCALS {
                return Err("local binding count exceeds 64".into());
            }
            self.expect(Token::Punct(b'='))?;
            let initializer = self.expr(0, 0)?;
            self.expect(Token::Punct(b';'))?;
            validate_ast_depth(&initializer)?;
            let constant = if kind == "val" {
                None
            } else {
                Some(eval_const_i64(&initializer)?)
            };
            let runtime_slot = self.local_initializers.len();
            if constant.is_none() {
                self.local_initializers.push(initializer);
            }
            // Bind only after the initializer passes admission, preventing
            // self and forward references and any hidden shadowing.
            self.bindings.push((name, constant, runtime_slot));
        }
        if self.current == Token::Word("let") {
            // A denied 'let' may still conceal a reserved actor operation,
            // e.g. 'let a = spawn Worker()'. Scan only this unsupported
            // initializer so actor capability diagnostics win without
            // changing unrelated ADT/match error precedence elsewhere.
            for _ in 0..MAX_NODES {
                match self.next() {
                    Err(message)
                        if message
                            .contains("actor and supervisor capabilities are unsupported") =>
                    {
                        return Err(message);
                    }
                    Ok(()) if matches!(self.current, Token::End | Token::Punct(b';' | b'}')) => {
                        break;
                    }
                    Err(_) => break,
                    Ok(()) => {}
                }
            }
            return Err(
                "E_LET_UNSUPPORTED: rebindable locals require typed SSA write lowering".into(),
            );
        }
        if self.current == Token::Word("mut") {
            return Err("E_MUT_QUALIFIER: bare mut requires an explicit binding qualifier".into());
        }
        if matches!(
            self.current,
            Token::Word(name) if self.bindings.iter().any(|(declared, _, _)| declared == name)
        ) {
            return Err("E_IMMUTABLE_REBIND: local const/val is not rebindable".into());
        }
        let mut trace_steps = Vec::new();
        let result = if self.current == Token::Word("using") {
            if !self.local_initializers.is_empty() {
                return Err("E_TELEMETRY_SCOPE: scoped telemetry with runtime local initializers is not lowered".into());
            }
            self.scalar_body(&mut trace_steps, &mut Vec::new())?
                .ok_or("scalar main requires a return")?
        } else {
            let inline_effects = self.inline_statements(0)?;
            self.expect(Token::Word("return"))?;
            let value = self.expr(0, 0)?;
            let result = if inline_effects.is_empty() {
                value
            } else {
                self.node(Expr::Sequence(inline_effects, Box::new(value)))?
            };
            self.expect(Token::Punct(b';'))?;
            self.expect(Token::Punct(b'}'))?;
            result
        };
        while self.current == Token::Word("export") {
            if export_count >= 128 {
                return Err("EXPORT_SOURCE_LIMIT: too many directives".into());
            }
            self.file_export()?;
            export_count += 1;
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
            locals: self.local_initializers,
            result,
            trace_steps,
        })
    }
    // The scalar profile supports one reachable return. Each preceding scope
    // either falls through before that return, or surrounds its evaluation.
    fn scalar_body(
        &mut self,
        trace_steps: &mut Vec<TraceStep>,
        aliases: &mut Vec<String>,
    ) -> Result<Option<Expr>, String> {
        let mut result = None;
        while self.take(Token::Word("using"))? {
            if result.is_some() || aliases.len() == 16 || trace_steps.len() >= 128 {
                return Err("E_TELEMETRY_SCOPE: unreachable scope or scope budget exceeded".into());
            }
            self.expect(Token::Word("telemetry"))?;
            self.expect(Token::Punct(b'.'))?;
            self.expect(Token::Word("span"))?;
            self.expect(Token::Punct(b'('))?;
            let Token::Quoted(raw) = self.current else {
                return Err("E_TELEMETRY_NAME: expected a span name literal".into());
            };
            let operation = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 256
                        && !s.contains('\\')
                        && !s.chars().any(char::is_control)
                })
                .ok_or(
                    "E_TELEMETRY_NAME: expected 1..256 UTF-8 bytes without escapes or controls",
                )?;
            self.next()?;
            self.expect(Token::Punct(b')'))?;
            self.expect(Token::Word("as"))?;
            let alias = self.name()?;
            if alias == "_"
                || aliases.contains(&alias)
                || self.parameters.contains(&alias)
                || self.bindings.iter().any(|(name, _, _)| name == &alias)
                || is_java_v1_reserved_parameter_name(&alias)
            {
                return Err(
                    "E_TELEMETRY_ALIAS: opaque span alias shadows an active binding".into(),
                );
            }
            self.expect(Token::Punct(b'{'))?;
            trace_steps.push(TraceStep::Begin(operation.to_owned()));
            aliases.push(alias);
            result = self.scalar_body(trace_steps, aliases)?;
            aliases.pop();
            if result.is_none() {
                if trace_steps.len() >= 128 {
                    return Err("E_TELEMETRY_SCOPE: scope budget exceeded".into());
                }
                trace_steps.push(TraceStep::End);
            }
        }
        if self.take(Token::Word("return"))? {
            let expression = self.expr(0, 0)?;
            validate_ast_depth(&expression)?;
            self.expect(Token::Punct(b';'))?;
            // A tail fallback after a returning using scope is unreachable.
            // Check its syntax and names without adding it to executable IR.
            if result.is_none() {
                result = Some(expression);
            }
        }
        self.expect(Token::Punct(b'}'))?;
        Ok(result)
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
pub(crate) fn validate_ast_depth(root: &Expr) -> Result<(), String> {
    let mut pending: Vec<(&Expr, usize)> = vec![(root, 1)];
    let mut counted_nodes = 0usize;
    while let Some((expr, level)) = pending.pop() {
        counted_nodes += 1;
        if counted_nodes > MAX_NODES {
            return Err("AST node budget exceeded".into());
        }
        if level > MAX_DEPTH {
            return Err("expression depth budget exceeded".into());
        }
        match expr {
            Expr::Literal(_) | Expr::Local(_) => {}
            Expr::Param(i) if *i < MAX_PARAMS => {}
            Expr::Param(_) => return Err("invalid scalar parameter index".into()),
            Expr::Seq(items) => {
                if items.is_empty() || items.len() > 16 {
                    return Err("E_SEQ_LIMIT: invalid scalar sequence arity".into());
                }
                for item in items.iter().rev() {
                    pending.push((item, level + 1));
                }
            }
            Expr::ScalarLoop(args, writable, condition, steps, locals, body) => {
                if args.len() > 8 || args.len() + locals.len() > 64 || writable.len() > args.len() {
                    return Err("E_UNIT_LIMIT: loop frame".into());
                }
                pending.push((condition, level + 1));
                pending.push((body, level + 1));
                for x in args.iter().chain(locals) {
                    pending.push((x, level + 1));
                }
                let mut blocks = vec![(steps.as_slice(), level + 1)];
                while let Some((items, d)) = blocks.pop() {
                    if d > MAX_DEPTH || items.len() > 64 {
                        return Err("E_UNIT_LIMIT: loop statements".into());
                    }
                    for step in items {
                        counted_nodes += 1;
                        if counted_nodes > MAX_NODES {
                            return Err("AST node budget exceeded".into());
                        }
                        match step {
                            LoopStep::Bind(x, _) | LoopStep::Set(_, x) | LoopStep::Exit(x, _) => {
                                pending.push((x, d + 1))
                            }
                            LoopStep::Loop(c, ss) => {
                                pending.push((c, d + 1));
                                blocks.push((ss, d + 1));
                            }
                        }
                    }
                }
            }
            Expr::ScopedCall(args, locals, body) => {
                if args.len() > 8 || args.len() + locals.len() > 64 {
                    return Err("E_UNIT_LIMIT: scoped helper frame".into());
                }
                pending.push((body.as_ref(), level + 1));
                for part in args.iter().chain(locals.iter()) {
                    pending.push((part, level + 1));
                }
            }
            Expr::Sequence(prefix, last) => {
                if prefix.len() > MAX_NODES {
                    return Err("AST node budget exceeded".into());
                }
                pending.push((last.as_ref(), level + 1));
                for part in prefix {
                    pending.push((part, level + 1));
                }
            }
            Expr::I32Checked(inner) => pending.push((inner.as_ref(), level + 1)),
            Expr::RefinementCast(inner, lo, hi) => {
                if lo > hi {
                    return Err("E_REFINEMENT_INTERVAL: invalid bounds".into());
                }
                pending.push((inner.as_ref(), level + 1));
            }
            Expr::Neg(inner) | Expr::ShiftRight8(inner) | Expr::InlineTask(inner) => {
                pending.push((inner.as_ref(), level + 1));
            }
            Expr::Choose(c, y, n) => {
                pending.push((c, level + 1));
                pending.push((y, level + 1));
                pending.push((n, level + 1));
            }
            Expr::Compare(op, a, b) => {
                if *op > 5 {
                    return Err("invalid comparison opcode".into());
                }
                pending.push((a, level + 1));
                pending.push((b, level + 1));
            }
            Expr::Add(left, right)
            | Expr::Sub(left, right)
            | Expr::Mul(left, right)
            | Expr::Div(left, right)
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

// Only typed, deterministic scalar expressions may initialize local const.
// Unlike static define const, a local cannot observe the process environment.
// This evaluator deliberately refuses function calls, including Integer.*.
fn eval_const_i64(expr: &Expr) -> Result<i64, String> {
    match expr {
        Expr::Choose(_,_,_) | Expr::Compare(_,_,_) => Err("E_NON_CONST_INITIALIZER: strict guard is not constexpr".into()),
        Expr::Literal(value) => Ok(*value),
        Expr::Seq(parts) => {
            // A constexpr sequence is permitted only when EVERY member is
            // proven constant, even if only its last result is retained.
            if parts.is_empty() || parts.len() > 16 {
                return Err("E_SEQ_LIMIT: invalid constexpr sequence".into());
            }
            let mut last = 0_i64;
            for part in parts {
                last = eval_const_i64(part)?;
            }
            Ok(last)
        }
        Expr::ScalarLoop(..) | Expr::ScopedCall(_,_,_) | Expr::InlineTask(_) | Expr::Sequence(_, _) => Err("E_NON_CONST_INITIALIZER: task execution is not constexpr".into()),
        Expr::Param(_) | Expr::Local(_) => Err(
            "E_NON_CONST_INITIALIZER: local const/type-first int requires proven constexpr".into(),
        ),
        Expr::I32Checked(_) | Expr::RefinementCast(_,_,_) | Expr::NamedRem(_, _) | Expr::NamedMod(_, _) | Expr::NamedPow(_, _)
        | Expr::BitXor(_, _) | Expr::BitAnd(_, _) | Expr::BitOr(_, _) => Err(
            "E_NON_CONST_INITIALIZER: function calls are not proven constexpr in this scalar profile".into(),
        ),
        Expr::Neg(expr) => eval_const_i64(expr)?
            .checked_neg()
            .ok_or_else(|| "E_CONST_OVERFLOW: signed i64 negation".into()),
        Expr::ShiftRight8(expr) => Ok(eval_const_i64(expr)? >> 8),
        Expr::Add(left, right) | Expr::Sub(left, right) | Expr::Mul(left, right) | Expr::Div(left, right) => {
            let lhs = eval_const_i64(left)?;
            let rhs = eval_const_i64(right)?;
            let value = match expr {
                Expr::Add(_, _) => lhs.checked_add(rhs),
                Expr::Sub(_, _) => lhs.checked_sub(rhs),
                Expr::Mul(_, _) => lhs.checked_mul(rhs),
                Expr::Div(_, _) if rhs == 0 => return Err("E_CONST_DIVZERO: signed i64 division".into()),
                Expr::Div(_, _) => lhs.checked_div(rhs),
                _ => unreachable!(),
            };
            value.ok_or_else(|| "E_CONST_OVERFLOW: signed i64 arithmetic".into())
        }
    }
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
        if token == Token::End {
            break;
        }
        window.rotate_left(1);
        window[5] = token;
        if matches!(
            &window[1..],
            [
                Token::Word("Mutex"),
                Token::Punct(b'<'),
                Token::Word(_),
                Token::Punct(b'>'),
                Token::Punct(b'(')
            ]
        ) {
            if window[0] == Token::Word("new") {
                return Err(
                    "E_LOCK_UNSUPPORTED: no compiled MutexLease or scheduler exists for Cranelift"
                        .into(),
                );
            }
            return Err(
                "E_NEW_REQUIRED: Mutex<T> is a nominal resource class; use new Mutex<int>(value)"
                    .into(),
            );
        }
        scan.next()?;
    }
    Ok(())
}

pub(crate) fn validate_source_envelope(text: &str) -> Result<(), String> {
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
    Ok(())
}

#[allow(dead_code)]
pub fn parse_verified_strict_i64(text: &str) -> Result<Program, String> {
    validate_source_envelope(text)?;
    let mut p = Parser::new(text)?;
    p.strict_fixed_arrays = true;
    quantity_scalar::parse_mode(p, true)
}

// Closed uniform unsigned scalar program. Its raw i64 literal storage is a
// bit pattern only; this wrapper cannot enter the signed frontend API.
pub(crate) struct UnsignedProgram(Program);
impl UnsignedProgram {
    pub(crate) fn inner(&self) -> &Program {
        &self.0
    }
}
pub(crate) fn parse_unsigned(text: &str) -> Result<UnsignedProgram, String> {
    validate_source_envelope(text)?;
    let mut p = Parser::new(text)?;
    p.unsigned_scalar = true;
    p.expect(Token::Word("pub"))?;
    p.expect(Token::Word("fnc"))?;
    p.expect(Token::Word("main"))?;
    p.expect(Token::Punct(b'('))?;
    fn ty(p: &mut Parser<'_>) -> Result<(), String> {
        if matches!(p.current, Token::Word("u64" | "uint")) {
            p.next()
        } else {
            Err("explicit u64 or uint required".into())
        }
    }
    if !p.take(Token::Punct(b')'))? {
        loop {
            let name = if matches!(p.current, Token::Word("u64" | "uint")) {
                ty(&mut p)?;
                p.name()?
            } else {
                let name = p.name()?;
                p.expect(Token::Punct(b':'))?;
                ty(&mut p)?;
                name
            };
            if p.parameters.len() >= MAX_PARAMS
                || p.parameters.contains(&name)
                || is_java_v1_reserved_parameter_name(&name)
                || matches!(name.as_str(), "u64" | "uint" | "int" | "main" | "Integer")
            {
                return Err("invalid unsigned parameter".into());
            }
            p.parameters.push(name);
            if !p.take(Token::Punct(b','))? {
                break;
            }
        }
        p.expect(Token::Punct(b')'))?;
    }
    if !p.take(Token::Punct(b':'))? {
        p.expect(Token::Punct(b'-'))?;
        p.expect(Token::Punct(b'>'))?;
    }
    ty(&mut p)?;
    p.expect(Token::Punct(b'{'))?;
    p.expect(Token::Word("return"))?;
    let result = p.expr(0, 0)?;
    p.expect(Token::Punct(b';'))?;
    p.expect(Token::Punct(b'}'))?;
    p.expect(Token::End)?;
    validate_ast_depth(&result)?;
    Ok(UnsignedProgram(Program {
        parameters: p.parameters,
        locals: vec![],
        result,
        trace_steps: vec![],
    }))
}

pub fn parse(text: &str) -> Result<Program, String> {
    validate_source_envelope(text)?;
    let original = Parser::new(text)?.program();
    // Only a complete, independently typed closed Option program can enter
    // the fused scalar lowering. Preserve all ordinary parser denials otherwise.
    if original.is_err()
        && text.contains("trap!")
        && text.contains("match")
        && let Ok(program) = closed_option::parse(text)
    {
        return Ok(program);
    }
    // Preserve authoritative declaration/effect/export diagnostics. Refine
    // only generic grammar denials (or the specific unsupported Struct case).
    if !original.as_ref().is_err_and(|err| {
        err.starts_with("unexpected token")
            || err.starts_with("unbound or unsupported identifier")
            || err.starts_with("expected scalar expression")
            || err.starts_with("invalid character")
            || err.starts_with("DECL_KIND_UNSUPPORTED: struct")
            || err.starts_with("E_LET_UNSUPPORTED")
    }) {
        return original;
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
        let mut previous = Token::End;
        while scan.current != Token::End {
            // A visibility qualifier pub[...] is not an indexed expression.
            if scan.current == Token::Punct(b'[') && previous != Token::Word("pub") {
                return Err("E_UNSUPPORTED_INDEX_WRITE: indexed Record/Map/Struct access requires checked target typing and guest lowering".into());
            }
            previous = scan.current;
            scan.next()?;
        }
    }
    original
}

#[cfg(test)]
mod tests {
    #[test]
    fn strict_refinement_admission() {
        let pre = "type Bounded = int where self >= 1 and self <= 42;";
        for expr in ["40+2", "84/2", "42 as Bounded"] {
            assert!(
                super::parse_verified_strict_i64(&format!(
                    "{pre} pub fnc main()->int {{return {expr};}}"
                ))
                .is_ok(),
                "{expr}"
            );
        }
        for expr in [
            "42/0",
            "-9223372036854775808 / -1",
            "9223372036854775807+1",
            "(9223372036854775807+1)*0",
            "43 as Bounded",
        ] {
            assert!(
                super::parse_verified_strict_i64(&format!(
                    "{pre} pub fnc main()->int {{return {expr};}}"
                ))
                .unwrap_err()
                .contains("E_VERIFY_UNPROVEN"),
                "{expr}"
            );
        }
    }
    use super::*;
    #[test]
    fn mutex_new_constructor_is_required_but_not_yet_runtime_supported() {
        // Constructor grammar applies to all simple nominal type arguments,
        // not just the currently executable scalar int. Nested type arguments
        // remain outside this narrowly bounded source admission gate.
        for ty in ["int", "bool", "float", "Buffer"] {
            let old = format!("pub fnc main():int {{ val counter = Mutex<{ty}>(0); return 0; }}");
            let canonical =
                format!("pub fnc main():int {{ val counter = new Mutex<{ty}>(0); return 0; }}");
            assert!(parse(&old).unwrap_err().contains("E_NEW_REQUIRED"), "{ty}");
            assert!(
                parse(&canonical)
                    .unwrap_err()
                    .contains("E_LOCK_UNSUPPORTED"),
                "{ty}"
            );
        }
        assert_eq!(
            eval(
                &parse("// Mutex<int>(0) is a comment\nfnc main():int{return 42;}")
                    .unwrap()
                    .result,
                &[]
            ),
            Ok(42)
        );
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
            assert!(
                error.contains("E_ACTOR_MEMORY_GUEST_UNSUPPORTED"),
                "{src}: {error}"
            );
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
                    error.contains(if expression.starts_with(&format!("{operation} ")) {
                        "OWNERSHIP_BANG_BACKEND_UNSUPPORTED"
                    } else {
                        "E_BANG_KEYWORD_WHITESPACE"
                    }),
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
            Expr::Local(_) | Expr::ScalarLoop(..) | Expr::ScopedCall(_, _, _) => {
                Err("test evaluator does not execute runtime local sequences")
            }
            Expr::Choose(c, y, n) => {
                if eval(c, args)? != 0 {
                    eval(y, args)
                } else {
                    eval(n, args)
                }
            }
            Expr::Compare(op, a, b) => {
                let a = eval(a, args)?;
                let b = eval(b, args)?;
                Ok(match op {
                    0 => a == b,
                    1 => a != b,
                    2 => a < b,
                    3 => a <= b,
                    4 => a > b,
                    5 => a >= b,
                    _ => return Err("invalid comparison"),
                } as i64)
            }
            Expr::InlineTask(body) => eval(body, args),
            Expr::Sequence(prefix, last) => {
                for part in prefix {
                    eval(part, args)?;
                }
                eval(last, args)
            }

            Expr::Neg(x) => eval(x, args)?.checked_neg().ok_or("overflow"),
            Expr::Add(a, b) => eval(a, args)?.checked_add(eval(b, args)?).ok_or("overflow"),
            Expr::Sub(a, b) => eval(a, args)?.checked_sub(eval(b, args)?).ok_or("overflow"),
            Expr::Mul(a, b) => eval(a, args)?.checked_mul(eval(b, args)?).ok_or("overflow"),
            Expr::I32Checked(x) => {
                let v = eval(x, args)?;
                i32::try_from(v).map(i64::from).map_err(|_| "i32 overflow")
            }
            Expr::RefinementCast(x, lo, hi) => {
                let v = eval(x, args)?;
                if v < *lo || v > *hi {
                    Err("refinement violation")
                } else {
                    Ok(v)
                }
            }
            Expr::Div(a, b) => {
                let left = eval(a, args)?;
                let right = eval(b, args)?;
                if right == 0 {
                    return Err("divisor_zero");
                }
                left.checked_div(right).ok_or("overflow")
            }
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
            Expr::Seq(parts) => {
                let mut result = None;
                for part in parts {
                    result = Some(eval(part, args)?);
                }
                result.ok_or("invalid_sequence")
            }
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
    fn exceptional_bang_keywords_fail_closed_before_clif() {
        for op in ["trap!", "throw!", "raise!", "panic!"] {
            let src = format!("fnc main():int{{return {op} value;}}");
            let error = parse(&src).expect_err("unlowered bang effect must fail");
            assert!(
                error.contains("EXCEPTION_BANG_BACKEND_UNSUPPORTED"),
                "unexpected diagnostic for {op}: {error}"
            );
        }
        assert!(parse("fnc main():int{return 7;} // throw!").is_ok());
    }

    #[test]
    fn eighteen_case_exception_bang_boundaries_never_forge_native_clif_effects() {
        // 18 spec #146 lexical vectors: six backend-denied canonical forms,
        // nine collision/legacy forms, one string and two comments.
        for expr in [
            "trap! parse_id(\"42\")",
            "trap!parse_id(\"42\")",
            "throw! Error(\"bad\")",
            "raise! Error(\"bad\")",
            "panic! Error(\"bad\")",
            "throw!Error(\"bad\")",
        ] {
            let source = format!("fnc main():int{{return {expr};}}");
            let err = parse(&source).expect_err("unimplemented effect must fail closed");
            assert!(
                err.contains(if expr.contains("! ") {
                    "EXCEPTION_BANG_BACKEND_UNSUPPORTED"
                } else {
                    "E_BANG_KEYWORD_WHITESPACE"
                }),
                "wrong diagnostic for {expr:?}: {err}"
            );
        }
        for expr in [
            "trap parse_id(\"42\")",
            "throw Error(\"bad\")",
            "raise Error(\"bad\")",
            "panic Error(\"bad\")",
            "trap !ready",
            "trap!=next",
            "trap!eq next",
            "trapped! parse_id()",
            "copy! owned",
        ] {
            let source = format!("fnc main():int{{return {expr};}}");
            if let Err(err) = parse(&source) {
                assert!(
                    !err.contains("EXCEPTION_BANG_BACKEND_UNSUPPORTED"),
                    "false exceptional bang in {expr:?}: {err}"
                );
            }
        }
        for source in [
            "fnc main():int{return 7;} // trap! throw! raise! panic!",
            "fnc main():int{/* trap! throw! raise! panic! */return 7;}",
            "fnc main():int{return \"trap! throw! raise! panic!\";}",
        ] {
            if let Err(err) = parse(source) {
                assert!(
                    !err.contains("EXCEPTION_BANG_BACKEND_UNSUPPORTED"),
                    "trivia falsely reclassified as control: {err}"
                );
            }
        }
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

    #[test]
    fn scoped_scalar_seq_model_preserves_source_order_and_rejects_invalid_arity() {
        for expression in [
            "seq(42)",
            "seq(1,2,42)",
            "(> 1,2,42)",
            "seq(1,seq(2,42))",
            "seq(1,(> 2,42))",
            "seq(1,2)+40",
            "seq(1,Integer.bitAnd(3,1),42)",
        ] {
            let source = format!("fnc main():int{{return {expression};}}");
            let parsed = parse(&source).expect("scoped scalar seq must parse");
            assert_eq!(eval(&parsed.result, &[]), Ok(42), "{source}");
        }
        for expression in [
            "seq()",
            "(>)",
            "seq(1,)",
            "(> 1,)",
            "seq(,1)",
            "seq(1,,2)",
            "seq(1;2)",
            "(1,2)",
            "seq 1,2",
        ] {
            let source = format!("fnc main():int{{return {expression};}}");
            assert!(
                parse(&source).is_err(),
                "{source}: invalid seq was admitted"
            );
        }
        let too_many = format!(
            "fnc main():int{{return seq({});}}",
            (0..17).map(|n| n.to_string()).collect::<Vec<_>>().join(",")
        );
        assert!(parse(&too_many).unwrap_err().contains("E_SEQ_LIMIT"));
        assert!(parse("fnc main(seq:int):int{return seq;}").is_err());
        let model_trap = parse("fnc main():int{return seq(9223372036854775807+1,42);}")
            .expect("arithmetic trapping sequence has valid syntax");
        assert_eq!(eval(&model_trap.result, &[]), Err("overflow"));
        let zero_divisor = parse("fnc main():int{return seq(Integer.rem(8,0),42);}")
            .expect("guarded remainder syntax");
        assert_eq!(eval(&zero_divisor.result, &[]), Err("divisor_zero"));
        // These Program expressions may also be constructed without parsing;
        // the recursive native emitter must never receive unsafe arities.
        assert!(validate_ast_depth(&Expr::Seq(vec![])).is_err());
        assert!(validate_ast_depth(&Expr::Seq(vec![Expr::Literal(1); 17])).is_err());
    }

    #[test]
    fn canonical_scalar_local_slots_distinguish_const_and_val() {
        let parsed = parse("fnc main(a:int):int{const c=2;val v=a+c;return v;}").unwrap();
        assert_eq!(parsed.locals.len(), 1);
        assert_eq!(parsed.result, Expr::Local(0));
        assert!(parse("fnc main(a:int):int{int c=a;return c;}").is_err());
    }

    #[test]
    fn inline_sync_task_scalar_semantics_and_fail_closed_admission() {
        for (source, arguments, expected) in [
            (
                "fnc main():int { do sync task {}; return 42; }",
                &[][..],
                42,
            ),
            (
                "fnc main():int { do sync task<void> { return; }; return 42; }",
                &[],
                42,
            ),
            (
                "fnc main():int { return sync task<int> { return 40+2; }; }",
                &[],
                42,
            ),
            (
                "fnc main(x:int):int { return 1 + sync /*ok*/ task<int> { return x; }; }",
                &[41],
                42,
            ),
            (
                "fnc main():int { return sync task { return sync task<int> { return 42; }; }; }",
                &[],
                42,
            ),
            (
                "fnc main():int { do sync task { do sync task<void> { return; }; return; }; return 42; }",
                &[],
                42,
            ),
            (
                "fnc main():int { do sync task { do sync task {}; return; } return 42; }",
                &[],
                42,
            ),
        ] {
            let p = parse(source).expect("non-suspending inline scalar task");
            assert_eq!(eval(&p.result, arguments), Ok(expected), "{source}");
        }
        for source in [
            "fnc main():int { return task<int> { return 42; }; }",
            "fnc main():int { return task /*c*/ <int> { return 42; }; }",
            "fnc main():int { do task<int> { return 42; }; return 0; }",
            "fnc main():int { return await task<int> { return 42; }; }",
            "fnc main():int { sync task {}; return 42; }",
            "fnc main():int { sync task<void> {}; return 42; }",
            "fnc main():int { return sync task {}; }",
            "fnc main():int { return sync task<void> { return; }; }",
            "fnc main():int { return sync task<int> {}; }",
            "fnc main():int { return sync task<int> { return; }; }",
            "fnc main():int { do sync task<int> {}; return 0; }",
            "fnc main():int { do sync task<int> { return 7; }; return 0; }",
            "fnc main():int { do sync task { return 7; }; return 0; }",
            "fnc main():int { do sync task { return sync task<int> { return 7; }; }; return 0; }",
            "fnc main():int { do sync task<void> { return 7; }; return 0; }",
            "fnc main():int { do sync task { await task<int> { return 1; }; } return 0; }",
            "fnc main():int { return sync task<int> { defer 1; return 42; }; }",
            "fnc main():int { return sync task<int> { task<int> {return 1;}; return 42; }; }",
            "fnc main():int { return sync task<int> { return 7; }; sync task {}; }",
        ] {
            assert!(
                parse(source).is_err(),
                "unsupported guest task admitted: {source}"
            );
        }
        for source in [
            "fnc main():int { do sync task<int> {}; return 42; }",
            "fnc main():int { do sync task<int> { return 1; }; return 42; }",
            "fnc main():int { do sync task { return 1; }; return 42; }",
            "fnc main():int { do sync task<void> { return 1; }; return 42; }",
        ] {
            let diagnostic = parse(source).expect_err("nonvoid do-sync must be rejected");
            assert!(
                diagnostic.contains("E_TASK_RETURN_TYPE"),
                "{source}: {diagnostic}"
            );
        }
        for source in [
            "fnc main():int { sync task {}; return 42; }",
            "fnc main():int { sync task<void> {}; return 42; }",
        ] {
            let diagnostic = parse(source).expect_err("bare inline-void task must be rejected");
            assert!(diagnostic.contains("E_SYNTAX"), "{source}: {diagnostic}");
        }
        let e = parse("fnc main(sync:int):int { return sync; }")
            .expect_err("reserved sync cannot be a parameter");
        assert!(e.contains("E_RESERVED"), "{e}");
        let good = parse("fnc main(sync_data:int):int { return sync_data; }").unwrap();
        assert_eq!(eval(&good.result, &[42]), Ok(42));
        let commented = parse("fnc main():int { /* do sync task<int> { } */ return 42; }").unwrap();
        assert_eq!(eval(&commented.result, &[]), Ok(42));
    }

    #[test]
    fn scoped_telemetry_is_bounded_and_does_not_hide_runtime_locals() {
        let program =
            parse("fnc main():int{using telemetry.span(\"outer\") as span{return 42;}}").unwrap();
        assert!(matches!(&program.trace_steps[..], [TraceStep::Begin(name)] if name == "outer"));
        let plain = parse("fnc main():int{return 42;}").unwrap();
        assert!(plain.trace_steps.is_empty());
        for source in [
            "fnc main(a:int):int{val x=a;using telemetry.span(\"s\") as span{return x;}}",
            "fnc main():int{using telemetry.span(\"\") as span{return 42;}}",
            "fnc main(a:int):int{using telemetry.span(\"s\") as a{return 42;}}",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
}

// Opt-in restricted durable Counter source admission. Use the existing
// lexer, not the scalar program parser, and consume the complete source.
// Reject comments/directives in this initial profile to avoid divergence.
pub fn durable_counter_descriptor(source: &str) -> Result<Vec<u8>, String> {
    let deny = || "E_DURABLE_COUNTER_ACTOR_ABI".to_owned();
    if source.len() > 16384
        || source
            .bytes()
            .any(|b| b >= 127 || (b < 32 && !b"\t\r\n".contains(&b)))
        || source.contains('/')
    {
        return Err(deny());
    }
    validate_source_envelope(source)?;
    let mut p = Parser::new(source)?;
    fn expect<'a>(p: &mut Parser<'a>, tokens: &[Token<'a>]) -> Result<(), String> {
        for &token in tokens {
            if !p.take(token)? {
                return Err("E_DURABLE_COUNTER_ACTOR_ABI".into());
            }
        }
        Ok(())
    }
    use Token::{End, Number, Punct, Word};
    expect(
        &mut p,
        &[
            Word("define"),
            Word("durable"),
            Word("actor"),
            Word("Counter"),
            Word("as"),
            Word("state"),
        ],
    )?;
    let field = match p.current {
        Word(v @ ("count" | "balance")) => v,
        _ => return Err(deny()),
    };
    expect(
        &mut p,
        &[
            Word(field),
            Punct(b':'),
            Word("int"),
            Punct(b'='),
            Number("0"),
            Punct(b';'),
            Word("receive"),
            Punct(b'('),
            Word("ActorMail"),
            Punct(b'<'),
            Word("int"),
            Punct(b'>'),
        ],
    )?;
    let mail = match p.current {
        Word(v @ ("mail" | "event")) => v,
        _ => return Err(deny()),
    };
    expect(&mut p, &[Word(mail), Punct(b')')])?;
    // The existing lexer represents the arrow as two punctuators. Require
    // contiguity before consuming them, not subtraction followed by >.
    if p.current != Punct(b'-') || source.as_bytes().get(p.pos) != Some(&b'>') {
        return Err(deny());
    }
    expect(
        &mut p,
        &[
            Punct(b'-'),
            Punct(b'>'),
            Word("void"),
            Punct(b'{'),
            Word("self"),
            Punct(b'.'),
            Word(field),
            Punct(b'='),
            Word("self"),
            Punct(b'.'),
            Word(field),
            Punct(b'+'),
            Word(mail),
            Punct(b'.'),
            Word("value"),
            Punct(b';'),
            Punct(b'}'),
            Word("end"),
        ],
    )?;
    if p.current != End {
        return Err(deny());
    }
    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        })
    }
    let mut sig = b"atto.actor.state.schema.v1\0".to_vec();
    for name in ["Counter", field] {
        sig.extend_from_slice(&(name.len() as u32).to_le_bytes());
        sig.extend_from_slice(name.as_bytes());
    }
    sig.push(1);
    let mut snapshot = b"ATTOST01\x01".to_vec();
    snapshot.extend_from_slice(&fnv(&sig).to_le_bytes());
    snapshot.extend_from_slice(&1u16.to_le_bytes());
    snapshot.push(1);
    snapshot.extend_from_slice(&0u64.to_le_bytes());
    snapshot.extend_from_slice(&fnv(&snapshot).to_le_bytes());
    let mut descriptor = vec![1, 7];
    descriptor.extend_from_slice(b"Counter");
    descriptor.push(field.len() as u8);
    descriptor.extend_from_slice(field.as_bytes());
    descriptor.extend_from_slice(&[1, 36]);
    descriptor.extend_from_slice(&snapshot);
    Ok(descriptor)
}

// Restricted schema-only profile, separate from executable actor lowering.
// Exact ASCII token stream, no comments/directives, bounded signed literals.
pub fn durable_scalar_schema(source: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let deny = || "E_DURABLE_SCALAR_SCHEMA_V1".to_owned();
    if source.len() > 16384
        || source
            .bytes()
            .any(|b| b >= 127 || (b < 32 && !b"\t\r\n".contains(&b)))
        || source.contains('/')
    {
        return Err(deny());
    }
    validate_source_envelope(source)?;
    let mut p = Parser::new(source)?;
    use Token::{End, Number, Punct, Word};
    fn take<'a>(p: &mut Parser<'a>, token: Token<'a>) -> Result<(), String> {
        if p.take(token)? {
            Ok(())
        } else {
            Err("E_DURABLE_SCALAR_SCHEMA_V1".into())
        }
    }
    fn name<'a>(p: &mut Parser<'a>) -> Result<&'a str, String> {
        const RESERVED: &str = " define pub private durable persistent actor as state end int bool true false self receive init fnc mtd let val mut const return await suspends susp async critical using with import namespace module class struct contract enum new spawn stop if else match on default void ";
        if let Word(n) = p.current
            && !n.is_empty()
            && n.len() <= 128
            && n.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !RESERVED.contains(&format!(" {n} "))
        {
            take(p, Word(n))?;
            return Ok(n);
        }
        Err("E_DURABLE_SCALAR_SCHEMA_V1".into())
    }
    fn sized(sig: &mut Vec<u8>, n: &str) {
        sig.extend_from_slice(&(n.len() as u32).to_le_bytes());
        sig.extend_from_slice(n.as_bytes());
    }
    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        })
    }
    for word in ["define", "durable", "actor"] {
        take(&mut p, Word(word))?;
    }
    let mut sig = b"atto.actor.state.schema.v1\0".to_vec();
    sized(&mut sig, name(&mut p)?);
    take(&mut p, Word("as"))?;
    let mut names = std::collections::BTreeSet::new();
    let mut tags = Vec::new();
    let mut payload = Vec::new();
    while p.current == Word("state") {
        if tags.len() >= 256 {
            return Err(deny());
        }
        take(&mut p, Word("state"))?;
        let field = name(&mut p)?;
        if !names.insert(field) {
            return Err(deny());
        }
        sized(&mut sig, field);
        take(&mut p, Punct(b':'))?;
        let tag = match p.current {
            Word("int") => {
                take(&mut p, Word("int"))?;
                1u8
            }
            Word("bool") => {
                take(&mut p, Word("bool"))?;
                2u8
            }
            _ => return Err(deny()),
        };
        take(&mut p, Punct(b'='))?;
        tags.push(tag);
        sig.push(tag);
        payload.push(tag);
        if tag == 1 {
            let neg = p.take(Punct(b'-'))?;
            let lit = match p.current {
                Number(n) => n,
                _ => return Err(deny()),
            };
            if lit.is_empty()
                || lit.len() > 19
                || (lit.len() > 1 && lit.starts_with('0'))
                || !lit.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(deny());
            }
            take(&mut p, Number(lit))?;
            let text = if neg {
                format!("-{lit}")
            } else {
                lit.to_owned()
            };
            let value = text.parse::<i64>().map_err(|_| deny())?;
            payload.extend_from_slice(&value.to_le_bytes());
        } else {
            let value = match p.current {
                Word("true") => 1,
                Word("false") => 0,
                _ => return Err(deny()),
            };
            let token = p.current;
            take(&mut p, token)?;
            payload.push(value);
        }
        take(&mut p, Punct(b';'))?;
    }
    take(&mut p, Word("end"))?;
    if p.current != End {
        return Err(deny());
    }
    let mut initial = b"ATTOST01\x01".to_vec();
    initial.extend_from_slice(&fnv(&sig).to_le_bytes());
    initial.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    initial.extend_from_slice(&payload);
    let checksum = fnv(&initial);
    initial.extend_from_slice(&checksum.to_le_bytes());
    Ok((initial, tags))
}

#[path = "scalar_library.rs"]
pub mod scalar_library;
