# Cranelift stage-0 scoped expression sequences (draft)

This is an **independent restricted native-JIT candidate**, not a general
AttoLang compiler or a sandbox for untrusted actor/tenant code.

Accepted scalar subset:

```atto
pub fnc main(): int { return seq! (1, 2, 42); }
pub fnc main(): int { return seq 1, 2, 42; }
pub fnc main(): int { return seq! 1, 2, 42; }
pub fnc main(): int { return (> 1, 2, 42); }
pub fnc main(): int { return seq! (1, (> 2, 42)); }
```

Each element is a checked signed-i64 expression evaluated **in source order**.
All non-final values are discarded, but a non-final overflow or a
`Integer.rem`/`Integer.mod` divide-by-zero **still returns the existing
checked native failure status**. The last value is the expression result.
Nested and single-element sequences are supported. Ordinary `(1,2)` is NOT a
sequence and is rejected by this scalar frontend. A sequence can contain no
more than 16 expressions; existing AST node/depth/source budgets apply.

2026-11-27 grammar revision: contiguous `seq!` selects the delimited sequence; ordinary `seq(...)` and `seq (...)` are calls and are rejected in this scalar profile, not executed as sequences. Bare sequences are admitted at the normal scalar main return boundary only. Caret return and warnings, general callable admission and other bare sequence contexts remain unimplemented.

Boundaries: no local declaration/assignment inside a sequence, no owned state,
actors, suspension, `defer`, host filesystem/network capabilities,
`Result`/`Option`, or arbitrary typed values. The AST validator is repeated
before native IR or JIT even for manually constructed `Program` objects.
Existing `Integer` intrinsics remain typed and checked.

The executable regression matrix `tests/seq_native_jit.rs` calls the actual
`orescranelift ir` and `orescranelift run` binary for positive output,
non-final traps, and rejected syntax. Separate frontend/backend unit tests
check the model, exact CLIF/JIT agreement and forged AST denial.

**Verification is pending** until `cargo test --locked --all-targets` runs on
the exact candidate head with a real admitted runner and records nonzero test
steps, source/head SHA, IR checks, real native JIT output and negatives.
A GitHub Actions job with no executed steps must not be called a compiler
failure or a guest conformance pass.

Cross-repo alignment: atto-spec #86, atto-platform-conformance #103,
atto-source.rs #407, atto-source.forge #42, atto-source.net #20.
