# Cranelift AOT-owned layout planner (compiler primitive)

**Status: Rust compiler infrastructure only; guest classes, heap ownership,
full AOT, tracing and browser execution are not implemented by this PR.**

The new `src/layout.rs` defines deterministic fixed-capacity layouts for plain
scalar fields. It is an independent prerequisite for future native aggregate
lowering and does not link Cranelift to Forge. It has no new dependencies.

| Mode | Representation | Typical intended workload |
| --- | --- | --- |
| AoS | Aligned fields within each fixed-stride record | Object-major iteration, heterogeneous field access |
| SoA / inverted class | Per-field contiguous columns, one stable numeric element index | Field-major scans, potential SIMD/vectorization |

The planner checks all offset additions, multiplication, padding and target
address-width limits **before an allocation or guest pointer exists**. It takes
an explicit maximum allocation-byte budget, bounds the number of scalar fields
at 1,024, and rejects invalid field/element indices. It supports 32-bit and
64-bit target bounds even when the compiler runs on a 64-bit host.

The planner handles plain 1-, 2-, 4- and 8-byte scalars only. It does not
represent borrowed references, owned graph edges, strings, arrays of variable
extent, object headers or destructor obligations. The allocator must separately
provide aligned memory, a single owner, generation-fenced handles, bounded
lifetime and deterministic teardown. Runtime resizing, migration between
layouts, class inversion and dropping are not implemented.

## Tracing versus sealed full AOT

`AccessCounts::suggested_layout` is deterministic compile-time feedback:
field-major accesses recommend SoA, otherwise AoS. **This is not a JIT tracer.**
A profiler may supply counts for a *subsequent build*, but active object
storage must never change its layout on the strength of an observation.
Actual in-process tracing, recompilation, deoptimization and safe object
migration need quiescent safepoints, ownership proofs and generation leases.
Strict sealed AOT must not fall back to JIT or an interpreter.

Native full AOT needs the independent object/emission/link path under draft
[Cranelift PR #83](https://github.com/ores-truffle-oreslang/oreslang-source.cranelift/pull/83).
The present stage-0 CLI continues to deny `aot` and `full-aot`. Nothing in
this planner makes that mode operational. Likewise, Java and LLVM parity is
not inferred from native Rust layout arithmetic.

## Runtime targets

- **Server and desktop:** native object and executable support is a separate
  future Cranelift compiler/linker milestone; OS ABI, deterministic destruction
  and sandbox boundaries still require native integration work.
- **Browser:** a browser does not execute arbitrary Cranelift-generated native
  machine code or in-process host JIT. Browser guest output must use the
  distinct `oreslang-source.wasm` WebAssembly compilation/runtime path, with
  shared source semantics and conformance tests. A 32-bit layout plan can
  inform portable ABI design; it is not executable browser support.
- **Backend WASM runtime:** separate host and guest compilation targets,
  resource permissions, memory model and actual WASM execution are required.

## Rust-only verification gates

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo test --locked --test layout_guest_admission
cargo clippy --locked --all-targets -- -D warnings
```

The Rust unit suite covers AoS/SoA offsets and alignment, 32-bit target
bounds, hard allocation quotas, arithmetic overflow, empty/oversized
shapes, out-of-bounds access, zero-capacity regions and static selection.
The real native CLI test must continue rejecting guest class/inverted-class,
actor, collection-factory, reflection and `eval` source in **both** `run`
and `ir`; a checked native scalar positive case remains executable.

All these checks are **NOT_EXECUTED until an exact-head Git source tree has
run them with nonzero CI steps**. Passing host layout tests, by itself, does
not certify actor zero-GC, Java/LLVM feature parity, AoS/SoA code generation,
runtime tracing, native AOT, or WASM/browser support.

Before readiness: attach exact source head/tree, executed Rust tests,
rustfmt/Clippy results, compiler/guest receipts and independent security
review. Keep this PR draft until the protected-main policy is effective.
