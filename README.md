# AttoLang Cranelift backend

**AttoLang** is the current public language name (formerly **Oreslang**).
The independent Cranelift JIT compiles a deliberately limited signed-i64 scalar subset, not full AttoLang.
This branding change does not imply full-language parity, AOT capabilities or
zero-GC guarantees. This backend's checked signed-i64 scalar subset has passed
real native execution tests using both `.atto` and legacy `.ores` files; new
fixtures for this verified subset SHOULD use `.atto`.
Do not rename repositories, dependencies, commands or fixed interfaces merely
to make their names match the public brand.


An **independent Rust + Cranelift** experimental backend for AttoLang. It is
**not** part of `oreslang-source.forge`. Forge remains on its own self-hosting
path, and must never link to Cranelift.

## Stage-0 executable scalar profile (validated `.atto` and `.ores`)

This backend compiles a deliberately narrow Oreslang source subset
into **actual Cranelift native JIT machine code** from `.atto` or `.ores` source, with a checked overflow
status rather than an unchecked wrap or process trap.

```oreslang
fnc main(x:int,y:int):int{return (x+2)*y;}
```

```sh
cargo run --locked -- run example.atto 7 8 # 72
cargo run --locked -- ir example.atto      # generated Cranelift IR
cargo test --locked --all-targets            # includes .atto/.ores execution parity
cargo test --locked --all-targets
```

Supported: one `main` function, 0–4 typed `int` parameters (`int x` or `x:int`),
signed i64 literals (including MIN and digit separators such as `1_000`),
grouping, unary +/-, binary + - *, and line comments.
The first additional Java-defined expression is `Integer.bitXor(a,b)` on signed64
values. Its arguments evaluate left-to-right and the compiler emits Cranelift's
native `bxor` instruction; caret operators remain rejected. Named `Integer.rem(a,b)`, `Integer.mod(a,b)`, and `Integer.pow(a,b)` also lower to native checked i64 code. Remainder follows truncation toward zero; modulus is nonnegative for either divisor sign. Zero divisors and negative exponents reject at execution. Power uses checked exponentiation by squaring.
Both return syntax variants `:int` and `->int` are accepted. Precedence and
arithmetic overflow match the checked-i64 scalar contract, not floating
point division. Source budget 1 MiB; AST 4096 nodes; nesting 128.

Rejected: **everything else**, including imports, helper functions, user-defined calls,
classes, actors, heaps, ownership, recursion, async, control flow, division, operator remainder expressions,
structural types, UTF-8 identifiers, and any trailing code.

The `%`, `^`, and `^^` operators are rejected at lexer admission. Typed
`Integer.bitXor` remains available.

## Explicit security boundary

Native JIT code executes inside this process. **It is not a sandbox** and
there is no guest actor heap isolation or untrusted-source support.
The entry ABI is internal (four i64 operands plus an output pointer returning
an i32 status), not a published AttoLang or foreign-call ABI.

Direct Cranelift crates are pinned to 0.137.0, and transitive dependencies
are pinned by the committed `Cargo.lock`, generated with Rust 1.99.1. The
`rust-toolchain.toml` pin and `--locked` CI prevent silent dependency drift.

The backend remains experimental. Passing scalar JIT tests does **not**
establish full Oreslang support, actor isolation or Forge self-hosting.

### Verification and dependency gates

- Generated Cranelift IR is verified **unconditionally** with
  `cranelift_codegen::verify_function` before defining or finalizing JIT
  native code. A deliberately malformed return-signature fixture must be
  rejected by the verifier; successful compilation alone does not demonstrate
  that the verifier ran.
- `scripts/check_lock_policy.py` fails on missing/bad SHA-256 registry checksums,
  git/path/alternate dependency sources, duplicate packages, drift from
  exactly pinned direct dependencies, and Cranelift release mismatches. The
  independent Python rejection tests must also execute in CI.
- Structural lockfile validation and `cargo test --locked` do **not** establish
  dependency vulnerability clearance or publisher provenance. Review applicable
  RustSec advisories and sign off separately before release.
- CI prints the actual checkout commit and root tree using Git objects, not
  handwritten source labels. A private external carrier must compare its
  **actual checked-out subtree** to the specified original source tree, prove
  that it ran test steps, and document its independent carrier identity; a
  matching printed label is insufficient.

## Acceptance

- Run native positive and negative cases, including overflow at runtime.
- Verify the signed64 `bit_xor` vectors against the normative Java #2193
  fixture through **actual Cranelift native guest execution**, not a host oracle.
  Do not reuse evidence from a different commit or tree. PR #1's Linux x64
  [successful exact-tree carrier run](https://github.com/honeypot-r-us-test/hnpt-e2e/actions/runs/40757732781)
  proves its *own* source tree (`1a5bcf7b94505144bbffbed8cb2aad09182ee203`),
  not later backend revisions.
- Require malformed/unsupported syntax to fail without stdout.
- Record exact source commit/tree, compiler versions, and executed CI steps.
- Cross-check source results against LLVM and Forge on their **actual** common
  scalar subset, never assume parity from shared fixture names.
- Do not mark any later PR as verified until its exact tree has admitted runner jobs with executed tests.

## Forge scalar/v1 intersection tests

The CLI integration suite also embeds the six positive and five negative
source examples from Forge's scalar/v1 contract. These are candidate
differential test cases; the repository does not yet pin a shared fixture
blob or provide executed cross-backend equivalence evidence.

## Fail-closed tenant and CLI admission

The Cranelift stage-0 compiler has **no actor runtime, IsolateVM, tenant
heap, supervisor CONTROL lane, ownership borrowing or untrusted native JIT
sandbox**. Actor/tenant constructs remain rejected in both `run` and
`ir` modes rather than being accepted as scalar aliases. The
`tenant_fail_closed` CLI integration suite probes those boundaries.

Before loading source or compiling any native code, `run` rejects more
than four runtime arguments (the fixed native entry ABI); `ir` rejects
all runtime arguments. This argument-vector admission is not a resource
quota on the host process and does not authorize hostile source execution.

## Unix source file admission

The CLI opens source descriptors nonblocking before validating that the
*opened descriptor* is a regular file. This rejects FIFO inputs, including
symlinks to FIFOs, in both `run` and `ir` without waiting for a writer.
A dedicated native CLI regression imposes a three-second hang guard.
Non-Unix systems retain the existing source-file handling behavior and do
not claim that nonblocking POSIX guarantee.

## Actor Sync v1 admission: unsupported on stage-0

The [Oreslang Actor Sync v1 draft conformance contract](https://github.com/ores-truffle-oreslang/oreslang-platform-conformance/pull/300)
requires source actors, isolated actor-owned state, non-reentrant turns, async
durable persistence and typed changefeed cursors. **This Cranelift backend has
none of those semantics yet.** As long as its compiler is scalar-only, it
must reject source actor and virtual sync constructs without emitting IR or a
guest executable.

`tests/actor_sync_admission.rs` adds 12 negative guest admission sources in
both `run` and `ir` CLI modes plus a positive scalar-with-comments guard.
These are **source rejection** tests and cannot be counted as AS01–AS24
guest implementation parity. Native actor IR, actor memory ownership and an
execution ABI must land first (issue #17). Cranelift is independent of Forge.

## ODB1 scalar floor — native signed shift by eight (draft)

For native Oreslang DB code, the canonical ODB1 encoder replaces
`int / 256` (a float-valued operation in Java/GraalVM Oreslang) with
a signed integer `>> 8`. Cranelift stage-0 now accepts **only**
`x >> 8`, with the literal shift count 8 and lower precedence than
checked addition/multiplication; all other shift counts, dynamic shift
expressions, and unsigned shifts fail closed before JIT lowering.

The actual Cranelift IR uses `sshr`, and the integration test runs
positive, negative and overflow vectors through the real native JIT
when cargo/runners are available. Stage-0 **still cannot** import the
DB's `wal_frame` module, allocate `List<int>`, construct `Frame`,
implement `Result<T,E>`, or offer actor-owned async WAL.
This is a *scalar prerequisite*, NOT five-backend ODB1 parity.

## Explicit full-AOT and mutable-state admission gate

`orescranelift aot` and `orescranelift full-aot` explicitly reject with nonzero
status rather than executing or packaging the stage-0 scalar JIT. Both `run`
and `ir` reject `@Inject`/`@StateBacked` state-provider annotations,
`define state container`, `define actor` and `define virtual actor`.
This is intentional: native heap provenance, scoped borrow lowering,
supervisor quiescence and deterministic object drop paths are unavailable.
Comment-only annotation text remains inert. This is **not** evidence of full
AOT, zero tracing GC or actor support. The negative gate verifies absence
of generated IR, guest stdout and build artifacts.

WebSocket, HTTP, TCP and LiveView actor declarations are an explicit
**unsupported** capability and emit a fail-closed diagnostic before Cranelift
IR or JIT execution. The CLI test matrix exercises eight declaration shapes
in both `run` and `ir`, plus unsupported `websocket.listen` calls and
comment-only positive controls. These tests execute through the native Rust/Cargo integration suite. This negative admission behavior is
not RFC6455 protocol support, actor ownership, WebSocket guest compilation,
or parity with Java, LLVM, WASM, Forge or .NET.



Experimental `isolate` mode applies a two-argument trusted scalar JIT to two separate tenant states. This is host lifecycle bookkeeping, not an untrusted native sandbox or source actor lowering. See docs/VM_ISOLATE_CONTRACT.md.
