# AttoLang compiler agent directives

New portable guest semantics, algorithms and test programs belong in canonical `.atto`; reserve host code for necessary compiler/AOT/ABI/security primitives. `.ores` remains explicit migration compatibility. Existing repository security and independent review requirements take priority.

## Strict guest AOT and zero-GC controlled-dynamism requirement

Review new code against [organization policy #38](https://github.com/ores-truffle-oreslang/.github/pull/38), [language-spec #18](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/18) and [this backend's policy](docs/AOT_ZERO_GC_DYNAMISM.md). **Sealed AOT rejects runtime eval, runtime class/method generation, unrestricted reflection, unknown proxy shapes and silent JIT/interpreter fallback before executable IR.** Statically generate proxies, virtual dispatch registries, generic/macro specializations, closures and async state machines. Require explicit ownership, `rt take`/`rt copy`/`rt borrow`/`rt share`, deterministic cleanup, bounded actor/task heaps, cycle handling, cancellation and generation fencing. No closure/function transport by channel/pub-sub.

Every PR must declare AOT profile and GC scope, justify host/compiler code vs `.atto`, record positive **and negative** `.atto` tests and exact-head executed guest receipts. Do not claim whole-process zero-GC from native source or a host-only test. Zero-step jobs are **NOT_EXECUTED**. Keep the no-Python/Rust-only tooling policy, security approvals and existing source-first rules intact.
