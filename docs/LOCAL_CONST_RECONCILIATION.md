# Cranelift local-binding semantic reconciliation (Atto v1)

**Authority:** atto-lang/atto-spec at `language/compile-time-constants-v1.md`, `language/local-binding-mutability-v1.md`, and `language/file-module-const-v1.md`. Source-local changes never override the canonical spec.

## Historical contradictory decisions

- Legacy ores-truffle-oreslang/oreslang-source.cranelift **#34** staged a contract in which local `const` allowed runtime-dependent initializers. This is **superseded** for Atto by the canonical local constexpr rule.
- Legacy **#21** correctly required local `const` constexpr but incorrectly classified unqualified type-first `int x = e` as runtime immutable. Atto classifies it as **constexpr**, exactly like `const int x = e`.
- Legacy **#129** stacks return-first callable syntax on #21. It is a *dependent feature*, not an independent local-binding semantics authority; do not copy its parser code here.
- File/module `define const ... = os.read_env("KEY")` is a carefully scoped runtime-once *static* exception, **not** a local `const` exception.

## Stage-0 native Cranelift subset

| Local spelling | Initializer | Rebind | Stage-0 |
| --- | --- | --- | --- |
| `const x = 42;`, `const int x = 42;` | deterministic compile-time scalar | no | positive |
| `int x = 42;` | deterministic compile-time scalar | no | positive |
| `val x = arg + 1;`, `val int x = arg + 1;` | eager runtime signed i64, checked | no | positive |
| `const x = arg;`, `int x = arg;` | runtime-dependent | no | compile rejection |
| `let x = ...;` | runtime, rebindable | yes | unsupported; reject |

Only a flat, bounded, signed-i64 entrypoint is implemented. Max 64 local bindings; no shadowing, nested scopes, class fields, actors, dynamic allocation, bool/string, runtime function calls in constexpr, arbitrary AOT, or TenantVM isolation claim. A direct native JIT is trusted-host code, **not a secure guest sandbox**.

Checked compile-time overflow rejects **before IR**, including unused constexpr locals. A `val` initializer is executed exactly once in lexical order; overflow in an unused `val` initializer still fails **at guest runtime**, with zero result output. The backend validates all AST references and limits independently of the parser.

## Overlap disposition

- Atto Cranelift **#52** modifies the scalar frontend/backend to support ordered sequence expressions. It is a **separate feature with overlapping files**; neither branch supersedes the other. Integrate and rerun native full-suite tests before merging either into a combined tree.
- **#50** uses a separate functional pipeline parser and constrained lexical `val` expansion, not these general local bindings. Verify composition explicitly; this PR does not claim it.
- Atto **#39** currently proposes `^` as integer XOR, but this conflicts with the later language decision reserving `^ expr;` for return and using `^^` for integer bitwise XOR. The checked-in bitwise spec still contains the old infix-XOR text. **Do not merge #39 or claim operator parity from this PR** until the canonical spec, parser, formatter and regression corpus agree.
- Do not privately mirror legacy archived source into another organization. This branch is an independent implementation in the active Atto repo.

## Admission

**Tests authored, not executed.** Required for promotion: `cargo fmt --all -- --check`, `cargo test --locked --all-targets`, `cargo clippy --locked --all-targets -- -D warnings`, plus exact-head real CLI `run`/`ir` positive, const-rejection and eager-overflow receipts. A failed zero-step GitHub Actions run is NOT_EXECUTED, not proof of native semantics.

## Cross-backend audit: legacy Oreslang versus canonical Atto

Read against the **merged Java source/compiler** and legacy LLVM/Wasm/.NET/Forge/Rust/JS branches, plus `atto-lang/atto-spec`. These are source/PR audits, NOT a single cross-backend compiled-guest parity receipt.

| Backend / evidence | Typed scalar local const/type-first | Untyped local const | Runtime val/let and availability |
| --- | --- | --- | --- |
| Java/Graal `oreslang-source.java#3066` (merged) | Stage-one constexpr checks, including type-right const | **Legacy runtime readonly allowed**, folded opportunistically | `val` runtime fixed, `let` mutable; Java preserved old implicit referent behavior in this merge |
| LLVM `oreslang-source.llvm#1779` (merged), `#1850` (open) | Typed local constexpr baseline, type-right parity still pending | Other scalar/inference PRs remain unverified | Native link/execute evidence must be pinned to exact heads |
| .NET `oreslang-source.net#516` (open), `#377` (merged) | Structured compiler draft enforces typed constexpr | **Java-compatible runtime readonly** retained in `#516` | Structured CFG/CIL and separate shipping/NativeAOT gates |
| Wasm `oreslang-source.wasm#400/#407` (merged) | Delegates compilation to pinned LLVM | Earlier guest positive explicitly permitted runtime `const` | Tests alone are not a separate Wasm language frontend |
| Forge `oreslang-source.forge#171` (open) | Bounded immutable i64 proposal, no proven full typed constexpr parity | Not established | Proposed native compile/guest testing; no full scalar mutability parity established |
| Rust `oreslang-source.rs#35` (open) | Immutable/type-first parser plus SSA drafts | Not established as matching current Atto const semantics | `let` implementation draft, source review/guest CI needed |
| JS `oreslang-source.js#96` (merged) | Typed lexical const/ESM source slice | Not established as current Atto semantics | Distinct typed program-v2 feature, no general ownership/actor support |
| Cranelift **this PR** | Implements compile-time `const int`, type-first `int`, and `const x: int` for signed i64 | **Strict constexpr** per Atto spec (intentionally differs from legacy Java stage one) | Eager native signed-i64 `val`, no `let` rebinding or secure sandbox |

### Authority and release policy

Canonical `atto-lang/atto-spec/language/compile-time-constants-v1.md` and `local-binding-mutability-v1.md` require **all local `const` initializers** to be compile-time-evaluable, including untyped `const x = e;`. This is a **migration difference**, not evidence that the merged Java stage-one runtime-`const` implementation is wrong for its historical profile. Never silently accept Java's compatibility behavior in a strict Atto profile or claim Java cross-backend parity based on syntax acceptance alone.

Canonical diagnostics should correspond to the Java/LLVM/.NET semantic classes: `E_NON_CONST_INITIALIZER`, `E_CONST_OVERFLOW`, `E_CONST_MUT_UNSUPPORTED`, `E_MUT_QUALIFIER`, and `E_IMMUTABLE_REBIND`. The Cranelift stage-0 `let` rejection is specifically an **unsupported feature**, not a prohibition in AttoLang.

Migration requires a versioned language profile and explicit runner evidence for older fixtures that admitted runtime `const`. Historical source docs are preserved rather than changed in place. The file/module `define const os.read_env` exception and contextual `if ... as const` never authorize local runtime constexpr elision.

Cross-backend exact-head tests, formatter alignment (`atto-format`, `oreslang-format`), native CLIF/LLVM IR, NativeAOT, Wasm isolation and negative-artifact evidence are still pending.

### Operator contract outside the binding scope (blocking overlap)

The checked-in Atto bitwise-v1 contract and Cranelift target #39 still propose infix `int ^ int` and Boolean `^^`. A newer explicit language decision instead sets `^ expr;` to be **return only**, `^^` to be **integer bitwise XOR**, `Math.pow()` as exponentiation, `<<<` and `>>>` as bitwise shifts, and `<<`/`>>` as directional stream/channel operations. The open `atto-spec#35` caret-return work is a related but not complete operator-contract reconciliation.

Consequences: do not use current main's infix-XOR clause as an authority for new code, do not silently reinterpret existing `^ ` guest fixtures, and keep `atto-source.cranelift#39` on an explicit conflict/integration hold. Source profile/version transition, conformance fixtures and formatter changes require their own coordinated work; this PR neither implements nor tests those operators. Existing Cranelift `>>8` signed-shift stage-0 behavior also must not be misrepresented as current Atto `>>` stream-read semantics.
