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

## Restricted inline `sync task` stage-0 candidate (draft)

The experimental Cranelift scalar frontend accepts `do sync task {}`
and `do sync task<void> { return; }` as sequential **void-only
inline statements** in synchronous scalar `main`. Omitted do-inline
type means `void`, never an inferred nonvoid payload.
`do sync task<int>` or `do sync task { return 42; }` rejects with
`E_TASK_RETURN_TYPE` before CLIF emission. Bare standalone
`sync task {};` and `sync task<void> {};` reject with `E_SYNTAX`:
the `do` keyword is required. Value-producing `sync task<int>
{ return <scalar-expression>; }` remains an expression of type `int`.
Inline bodies execute in lexical order on the caller, without spawning
a child or creating `Future<T>`. Checked arithmetic failures in an
inline value expression suppress guest result publication. Only
signed-i64 arithmetic, parameters and nested non-suspending inline
tasks are within this experimental subset.

**Not supported:** independently scheduled `task<T>`, `await task`,
suspension, `Future.cancel()`, mutable locals/captures, child TaskScopes,
defer/using, scope guards, or tenant scheduling guarantees. These
require checked lowering and remain rejected before CLIF or executable
guest emission. This stage-0 JIT is trusted, **not** a sandbox for
untrusted tenant code. See atto-spec PR #129 for the full target contract
and atto-platform-conformance PR #115 for portable fixtures. The
compiler subset does not pass those full portable fixtures yet.

The source includes parser tests and real CLI native-guest tests in
`tests/inline_sync_task.rs`. For executed checks, precise source commit/tree,
runner logs, and outcomes, see the current audit evidence on AttoLang
Cranelift PR #54; run `cargo test --locked --all-targets --no-fail-fast`
on the exact head. These receipts can certify only the restricted trusted
scalar subset, not full AttoLang task/cancellation/actor capabilities.
The repository capability inventory remains **NOT_CERTIFIED**.
