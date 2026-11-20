# CRANELIFT — full guest AOT and zero-GC controlled dynamism

**TARGET documentation; no executable guest parity certified.** See [organization directive #38](https://github.com/ores-truffle-oreslang/.github/pull/38) and [canonical language contract #18](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/18).

## Compile-time code policy

- Select and declare JIT, interpreter, hybrid, host-only AOT, **sealed full guest AOT**, or separately governed **precompiled AOT module** mode. Sealed mode builds every reachable guest target before running, and rejects eval/source compilation, new runtime classes/methods, monkey-patching, arbitrary reflection and silent interpreter/JIT fallback **before emitting executable code**.
- Keep valid inheritance, virtual calls, typed generics/typeclasses, macros, closures and async state machines via finite build-time registries, metadata and lowering. Proxy wrappers mirror methods of a statically known type/interface, enforcing visibility, receiver type, static-vs-instance separation, effects and ownership. Generate forwarders at compile time, not at runtime.
- Every guest allocation has provable owner/region and exactly-once reclamation across return, `await`, cancellation, supervisor restart, I/O and hot-generation retirement. Long-lived actors must release temporary allocations while active. Strong-reference cycles must have safe weak/region handling; quota/OOM must be bounded and fail closed.
- Channels/pub-sub move/copy/share Sendable data only, not functions, closures, raw pointers or borrowed actor state. Preserve no implicit conversions/operator overloading, no `%`/`^`/`^^`, and no silent nested `Future`/`Option` flattening.
- **Do not confuse zero guest tracing-GC with process zero-GC**, or static host compilation with full guest AOT. AOT itself does not prove hard real-time bounded cleanup.

## Backend consideration

Cranelift is a machine-code backend but can be hosted in JIT configurations; strict sealed mode must emit the whole guest executable ahead of execution with no runtime codegen or interpreter fallback. Independently demonstrate static reachability, native drop and actor/continuation lifetime tests.

## Required tests and evidence

- **Compile/execute positive:** known-class proxy with virtual and static dispatch, compile-time macro/generic specialization, actor-owned allocation and closure/task-frame move across suspension.
- **Reject before IR:** eval, unknown proxy/reflection target, private escalation, missing dispatch graph edge, escaping borrow, function sent via channel, forbidden strong cycles, unapproved module loading, implicit JIT fallback.
- **Stress:** long-lived actor churn and quota/OOM, cancellation/restart plus pending native I/O, stale generation, double drop and deterministic destructor behavior.
- **Receipt:** exact source/head/tree/fixture blob SHA, true compiler and generated artifact SHA256, actual guest output, nonzero executed steps and allocator/GC instrumentation. Host-only model, parser-only fixture or `steps=[]`/`steps=null` is NOT_EXECUTED for guest parity.

This file does not change compiler behavior or assert feature completion. Keep all guest code canonical `.atto` and preserve Rust-only tooling, security and independent reviewer gates.
