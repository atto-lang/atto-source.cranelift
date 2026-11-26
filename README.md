> **AI agent instructions (organization-wide):** Before working on code, issues, or pull requests, read and follow the [organization `AGENTS.md`](https://github.com/atto-lang/.github/blob/main/AGENTS.md), including its **merge-first stealth-development policy**. Honor any repository-local `AGENTS.md`, explicit holds, and GitHub-enforced rules.

# atto-source.cranelift

Cranelift-based code generation and native compilation backend for AttoLang.

## Naming and specification migration

This is the canonical [AttoLang backend repository](https://github.com/atto-lang/atto-source.cranelift). Use `.atto` for portable AttoLang source and update local remotes to `https://github.com/atto-lang/atto-source.cranelift.git`; do not use historical repository links or naming in new examples.

- **Normative grammar and semantics:** [atto-spec](https://github.com/atto-lang/atto-spec). Check the current versioned contracts for ownership operations (`borrow!`, `take!`, `share!`, `copy!`, `copy!!`), implicit argument borrowing, `lt!` binders, `eq`/`neq` value equality, `susp` effects and file/module `define const`. These are *specification targets*, not claims of executable support in this backend.
- **Positive and negative guest execution evidence:** [atto-platform-conformance](https://github.com/atto-lang/atto-platform-conformance) and [atto-compilers-parity-testing](https://github.com/atto-lang/atto-compilers-parity-testing). Report `passed`, `failed`, `unsupported` or `not executed`, and pin exact compiler, source tree, target and toolchain.
- **Portable shared sources and target constraints:** [atto-common-source](https://github.com/atto-lang/atto-common-source) and [atto-target-profiles](https://github.com/atto-lang/atto-target-profiles). Keep backend-specific gaps explicit, and do not infer guest parity from host tests or zero-step CI runs.

## Scope

This repository is the AttoLang ecosystem home for:

- Native code generation experiments, runtime ABI integration, and comparison against primary backends.
- Compatibility with the [canonical language specification](https://github.com/atto-lang/atto-spec) where applicable.
- Evidence-based tests, benchmark results, and cross-backend conformance reporting.

## Architecture and compatibility

AttoLang is designed around actors, explicit ownership, and portable concurrency semantics. Backend-specific optimizations must not silently change the shared language contract. Runtime capabilities, target restrictions, and unsupported features should be documented explicitly.

## Building and testing

Consult the checked-in source tree and CI workflows for verified commands and supported targets. Add target-specific build instructions and exact-head execution results as they become available; do not treat a passing host-unit test as proof of guest-language conformance.

## Project status

This README describes the repository's intended responsibilities. Feature support and release readiness must be established from implementation, tests, and CI evidence rather than this description alone.

## Related repositories

- [AttoLang organization](https://github.com/atto-lang)
- [Language specification](https://github.com/atto-lang/atto-spec)
- [Platform conformance](https://github.com/atto-lang/atto-platform-conformance)
- [Commercial infrastructure](https://github.com/atto-co)

## Repository ownership and contribution routing

**Belongs here:** Cranelift code generation, ABI lowering, compiled guest smoke tests, runtime integration and independent backend differential evidence.

**Belongs elsewhere:** LLVM-specific IR and codegen in atto-source.llvm; other backend implementations in their respective atto-source.* repositories.

**Security and portability boundary:** Keep source semantics/ownership checks ahead of codegen; contain actor traps and memory bounds, prevent stale generation handles, and require native guest receipts.

AttoLang is the canonical project identity. Use `.atto` source and `atto-*` naming for all new documentation and portable library code. This README defines scope, not feature readiness.

**Acceptance:** name exact source SHA/tree, runtime/toolchain, positive and negative checks and execution receipt before claiming support. Host-only tests and empty CI steps do not prove guest execution.


## Imported executable stage-0 scalar compiler (not complete AttoLang)

The canonical repository now contains the original **Rust/Cranelift** experimental signed-64-bit scalar JIT copied from [oreslang-source.cranelift](https://github.com/ores-truffle-oreslang/oreslang-source.cranelift) at source commit `d4acaf1dc36117b09d6a765665c17d55bea6c18b`. The copied compiler and Rust tests retain historical `orescranelift` binary identifiers and `.ores` fixture bytes for compatibility; `.atto` scalar compilation is exercised by Rust tests. The portable language contract is [atto-spec](https://github.com/atto-lang/atto-spec).

To attempt the exact-head native checks on a Rust-enabled host:

```sh
cargo test --locked --all-targets
cargo run --locked -- run tests/fixtures/clients-scalar-v1/01-precedence.ores
cargo run --locked -- ir tests/fixtures/clients-scalar-v1/01-precedence.ores
```

**Evidence status:** these commands are test instructions, not a claim they ran successfully in this canonical repository. The [native GitHub Actions workflow](.github/workflows/native-cranelift.yml) must show assigned jobs, nonzero executed steps, native guest results and an exact source SHA before recording any capability as passed. A mirror in another private organization requires the same blob SHA proof and explicit runner logs. The repository inventory remains `NOT_CERTIFIED` with `not_executed` statuses until then.

This compiler does **not** currently support full AOT, actor-language compilation, untrusted-isolate sandboxing, ownership-bang lowering, the complete Atto parser, or five-backend parity. It is a stage-0 **trusted** scalar JIT, not a substitute for LLVM/Forge/Java backend verification. No Python scripts or test harnesses are imported.


## Canonical caret/XOR scalar JIT candidate (draft, not release-certified)

The historical scalar-v4 corpus (Git blob `8185a1b4bf0e0a776b4168cad4805fc40660327d`) assigned integer XOR to infix `^` and rejected `^^`. That is **not** current AttoLang semantics. It remains byte-identical in `contracts/parity/portable-scalar-v4-20261119.json` as history only.

The draft canonical scalar-v5 corpus in `contracts/parity/portable-scalar-v5-20261120.json` instead requires `^ 42;` as statement-leading return and contiguous `11 ^^ 4` as integer XOR (result `15`); `11 ^ 4`, spaced/comment-separated carets and the obsolete `%` language operator must fail before JIT IR. The new real-JIT guest tests in `tests/portable_scalar_v5.rs` cover 13 positives, four source rejects, four overflow cases, plus direct caret-return and no-IR negative checks. Historical v1/v2/v4 operator outcomes are source-only archival checks, not current execution evidence.

This is an **i64-only, limited stage-0 compiler**. Its bounded signed `ashr 8` facility uses canonical spelling; the obsolete arithmetic `>> 8` spelling now rejects. Other shift counts, triple shifts and typed stream I/O are not supported by this slice. The [November 23 numeric decision](https://github.com/atto-lang/atto-spec/blob/main/language/cross-backend-semantics-reconciliation-v1.md) supersedes the historical scalar-v5 manifest's float-quotient sentence: signed `int / int` truncates toward zero. Native `sdiv` is guarded for zero divisors and MIN/-1, with distinct existing scalar status codes. The added native CLI tests are authored, not yet executed at this head. Recoverable guest Error/Result/trap and floating signatures remain unsupported; native status codes are not guest error ABI certification. The historical manifest is retained unchanged.

These changes are authored in [Cranelift #39](https://github.com/atto-lang/atto-source.cranelift/pull/39). **No exact-head Cargo/JIT guest execution has been observed** after these edits, and no release support is claimed. Do not mark any corresponding [11-backend conformance #236](https://github.com/atto-lang/atto-platform-conformance/pull/236) cells as passing without authenticated execution receipts. Preserve security-governed CI admission and independent review; keep draft.


### Experimental numeric Result entry (candidate, NOT EXECUTED)

The stage-0 entry may explicitly return `Result<int, Error>` using
`return trap! (a / b);` or `return trap (a / b);`. Both evaluate the
parenthesized scalar expression once. The generated native entry writes three
i64 slots (tag, payload, error code): Ok uses tag 0 and code 0; Err uses tag 1,
zero payload, and candidate identity 65537 (ArithmeticOverflowError) or 65538
(DivisionByZeroError). Native entry status 0 means a completed tagged result,
including Err; unknown statuses/tags fail closed. CLI renders Ok(value) or
Err(code) for evidence, not a standardized wire format. Scalar entry ABI and
isolate path remain separate. General Result constructors, Option, nested
traps, arbitrary guest throw/raise/panic, callable propagation, cleanup,
postfix ?, floating values and Integer.pow inside trap are unsupported.
The latter has a third error category without an agreed numeric identity.
Do not certify this narrow candidate as full guest error or backend parity.

Bang-suffixed Atto keyword operators require whitespace after the complete suffix (including `copy!!`). Rust-style `trap!(...)` is rejected; no whitespace is allowed between the word and its bang. This scalar lexer still refuses unsupported ownership, actor and macro operators.
