# Owned scalar regions for future Cranelift AOT actor heaps

**Prototype status:** host Rust storage only, not guest/actor lowering, a
production sandbox, or demonstrated whole-process zero-GC. This branch is
stacked on the [AoS/SoA layout planner PR #126](https://github.com/ores-truffle-oreslang/oreslang-source.cranelift/pull/126).

## Admission and lifecycle

- The owner supplies a previously validated `LayoutPlan` and its own upper
  byte budget. Both the compile-time plan and this runtime allocation enforce
  the budget. The runtime checks `usize` conversion and uses fallible
  `try_reserve_exact` before initialization.
- The buffer is fixed-capacity and zero-initialized. There is no tracing
  collector, relocation, resize, pointer-valued guest reference or finalizer.
- A private per-owner `Arc<()>` authority and generation are embedded in
  `RegionLease`. A foreign or stale lease cannot read/write another region,
  including one created after its predecessor retired.
- All access goes through checked field/index offsets in the compile-time
  planner. The current data bridge admits only signed i64 fields and stores
  little-endian bytes, avoiding unaligned host pointer casts.
- Rust borrow checking requires `&mut OwnedScalarRegion` for mutation; the
  immutable read path has only `&OwnedScalarRegion`. This does not implement
  guest `critical-read`/`critical-write`, actor locks or suspension checks.
- `retire()` closes admission, logically clears the data and drops the buffer
  allocation immediately. Cloned lease objects retain only authority identity,
  not the data. The `Drop` path also clears logically and frees the buffer.
  **Neither operation is certified cryptographic zeroization.**

### Memory and security limits

The `Vec<u8>` allocation has only byte alignment. All scalar I/O is copied
through bytes, so this prototype deliberately **must not** pass its underlying
pointer to Cranelift-generated native typed loads/stores. Production native
code requires an appropriately aligned allocation and a verified ABI for
bounds, provenance, generation, and actor suspension. No pinning of outstanding
native guest calls exists here; the embedding scheduler must enforce quiescence
before retiring physical memory.

Zero guest tracing-GC is an intended design constraint, not a proven guest
runtime guarantee. Rust host allocations, control structures and the JIT
compiler are not covered by a whole-process zero-GC claim. No cross-actor
channel transport, shared mutable state, foreign pointers or refcounted
object graphs are admitted by this prototype.

## Verification (Rust-only)

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

The six new Rust unit tests exercise each AoS/SoA mode against native-byte
round-trips and signed64 edge values, foreign lease rejection, post-retirement
rejection, generation/replacement identity, independent quota enforcement,
scalar-type mismatch and zero-capacity access.

These are **host runtime unit tests**, not compiled guest execution. For a
real guest parity milestone, connect typed AttoLang aggregate/actor AST and
ownership checker to CLIF memory operations, build and execute generated
native AOT in an actual target, test non-overlap with native sanitizers, and
validate a shared source fixture independently against Java/LLVM. Browser
compatibility requires WASM guest compilation; Rust host storage does not
establish browser execution.

Do not mark this implementation verified until exact-head Rust tests have
actually executed with nonzero CI steps. Keep stacked PRs draft until review
and protected-main gates are met.
