# Cranelift local-binding semantic reconciliation (Atto v1)

**Authority:** atto-lang/atto-spec at \`language/compile-time-constants-v1.md\`, \`language/local-binding-mutability-v1.md\`, and \`language/file-module-const-v1.md\`. Source-local changes never override the canonical spec.

## Historical contradictory decisions

- Legacy ores-truffle-oreslang/oreslang-source.cranelift **#34** staged a contract in which local \`const\` allowed runtime-dependent initializers. This is **superseded** for Atto by the canonical local constexpr rule.
- Legacy **#21** correctly required local \`const\` constexpr but incorrectly classified unqualified type-first \`int x = e\` as runtime immutable. Atto classifies it as **constexpr**, exactly like \`const int x = e\`.
- Legacy **#129** stacks return-first callable syntax on #21. It is a *dependent feature*, not an independent local-binding semantics authority; do not copy its parser code here.
- File/module \`define const ... = os.read_env("KEY")\` is a carefully scoped runtime-once *static* exception, **not** a local \`const\` exception.

## Stage-0 native Cranelift subset

| Local spelling | Initializer | Rebind | Stage-0 |
| --- | --- | --- | --- |
| \`const x = 42;\`, \`const int x = 42;\` | deterministic compile-time scalar | no | positive |
| \`int x = 42;\` | deterministic compile-time scalar | no | positive |
| \`val x = arg + 1;\`, \`val int x = arg + 1;\` | eager runtime signed i64, checked | no | positive |
| \`const x = arg;\`, \`int x = arg;\` | runtime-dependent | no | compile rejection |
| \`let x = ...;\` | runtime, rebindable | yes | unsupported; reject |

Only a flat, bounded, signed-i64 entrypoint is implemented. Max 64 local bindings; no shadowing, nested scopes, class fields, actors, dynamic allocation, bool/string, runtime function calls in constexpr, arbitrary AOT, or TenantVM isolation claim. A direct native JIT is trusted-host code, **not a secure guest sandbox**.

Checked compile-time overflow rejects **before IR**, including unused constexpr locals. A \`val\` initializer is executed exactly once in lexical order; overflow in an unused \`val\` initializer still fails **at guest runtime**, with zero result output. The backend validates all AST references and limits independently of the parser.

## Overlap disposition

- Atto Cranelift **#52** modifies the scalar frontend/backend to support ordered sequence expressions. It is a **separate feature with overlapping files**; neither branch supersedes the other. Integrate and rerun native full-suite tests before merging either into a combined tree.
- **#50** uses a separate functional pipeline parser and constrained lexical \`val\` expansion, not these general local bindings. Verify composition explicitly; this PR does not claim it.
- Atto **#39** adjusts scalar-v4 parity; retain its \`^ \` integer-XOR semantic migration and do not restore the old Oreslang caret ban.
- Do not privately mirror legacy archived source into another organization. This branch is an independent implementation in the active Atto repo.

## Admission

**Tests authored, not executed.** Required for promotion: \`cargo fmt --all -- --check\`, \`cargo test --locked --all-targets\`, \`cargo clippy --locked --all-targets -- -D warnings\`, plus exact-head real CLI \`run\`/\`ir\` positive, const-rejection and eager-overflow receipts. A failed zero-step GitHub Actions run is NOT_EXECUTED, not proof of native semantics.
