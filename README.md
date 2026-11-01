# Oreslang Cranelift backend

An **independent Rust + Cranelift** experimental backend for Oreslang. It is
**not** part of `oreslang-source.forge`. Forge remains on its own self-hosting
path, and must never link to Cranelift.

## Stage-0 executable scalar profile

This backend compiles a deliberately narrow Oreslang source subset
into **actual Cranelift native JIT machine code**, with a checked overflow
status rather than an unchecked wrap or process trap.

```oreslang
fnc main(x:int,y:int):int{return (x+2)*y;}
```

```sh
cargo run --locked -- run example.ores 7 8 # 72
cargo run --locked -- ir example.ores      # generated Cranelift IR
cargo test --locked --all-targets
```

Supported: one `main` function, 0–4 typed `int` parameters (`int x` or `x:int`),
signed i64 literals (including MIN and digit separators such as `1_000`),
grouping, unary +/-, binary + - *, immutable scalar locals, and line comments.
The first additional Java-defined expression is `Integer.bitXor(a,b)` on signed64
values. Its arguments evaluate left-to-right and the compiler emits Cranelift's
native `bxor` instruction; caret operators remain rejected. Other `Integer.*`
named functions (including `rem`, `mod`, and `pow`) remain unsupported.
Both return syntax variants `:int` and `->int` are accepted. Precedence and
arithmetic overflow match the checked-i64 scalar contract, not floating
point division. Source budget 1 MiB; AST 4096 nodes; nesting 128.

Rejected: **everything else**, including imports, helper functions, user-defined calls,
classes, actors, heaps, ownership, recursion, async, control flow, division, dynamic/zero-divisor remainder,
structural types, UTF-8 identifiers, and any trailing code.

The Oreslang language contract removes the `%`, `^`, and `^^` operators. The
Cranelift frontend rejects `%` before native code emission; named remainder
functions are still unsupported and must not be silently substituted.

## Explicit security boundary

Native JIT code executes inside this process. **It is not a sandbox** and
there is no guest actor heap isolation or untrusted-source support.
The entry ABI is internal (four i64 operands plus an output pointer returning
an i32 status), not a published Oreslang or foreign-call ABI.

Direct Cranelift crates are pinned to 0.137.0, and transitive dependencies
are pinned by the committed `Cargo.lock`, generated with Rust 1.99.1. The
`rust-toolchain.toml` pin and `--locked` CI prevent silent dependency drift.

This branch remains experimental. Passing its scalar JIT tests does **not**
establish full Oreslang support, actor isolation or Forge self-hosting.

## Acceptance

- Run native positive and negative cases, including overflow at runtime.
- Verify the four signed64 `bit_xor` vectors against the normative Java #2193
  fixture through **actual Cranelift native guest execution**, not a host oracle.
  The tests have been authored on a draft branch; no exact-head executed CI
  result has yet been confirmed for this change.
- Require malformed/unsupported syntax to fail without stdout.
- Record exact source commit/tree, compiler versions, and executed CI steps.
- Cross-check source results against LLVM and Forge on their **actual** common
  scalar subset, never assume parity from shared fixture names.
- Keep the branch draft until CI actually allocates a runner and executes tests.

## Forge scalar/v1 intersection tests

The CLI integration suite also embeds the six positive and five negative
source examples from Forge's scalar/v1 contract. These are candidate
differential test cases; the repository does not yet pin a shared fixture
blob or provide executed cross-backend equivalence evidence.

## Unix source file admission

The CLI opens source descriptors nonblocking before validating that the
*opened descriptor* is a regular file. This rejects FIFO inputs, including
symlinks to FIFOs, in both `run` and `ir` without waiting for a writer.
A dedicated native CLI regression imposes a three-second hang guard.
Non-Unix systems retain the existing source-file handling behavior and do
not claim that nonblocking POSIX guarantee.

## Shared Java/LLVM/Wasm/.NET/Cranelift scalar intersection

The exact-byte shared contract `contracts/parity/portable-scalar-v1-20261101.json`
has Git blob `001e8894cbeb87509878a4de36263a1a327805b1`, matching the
corresponding candidate in Java, LLVM, Wasm and .NET. It contains **16 positive
checked-i64 expressions and 6 compiler-negative admission vectors**. The
`tests/portable_scalar_v1.rs` integration runner pins the contract blob,
compiles each expression with the Cranelift frontend and executes the real
native JIT for positive cases. Negative cases must fail without stdout and
have a compiler-owned diagnostic. No independent Rust arithmetic is used
to credit guest execution.

Eleven additional `tests/parity/34-*.ores` through `44-*.ores` guest files are
byte-identical (same Git blob SHA per file) to programs 34–44 in
`oreslang-compilers-parity-testing` draft PR #16. The
`tests/portable_scalar_intersection.rs` runner verifies each pinned blob
before compiling and executing the source through the Cranelift CLI.
Neither these test bodies nor the pinned manifest alone establish
five-backend execution parity.

The removed `%` operator is rejected by the Cranelift lexer, including in
unused immutable initializers. The shared JSON negative contract likewise
rejects `%`, `^`, `^^`, unsupported integer division and out-of-range
literals before native publication. Do not reintroduce the legacy remainder
branch to make a test green.

**Evidence required:** an exact-head Actions job with nonzero executed steps,
`cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D
warnings`, `cargo test --locked --all-targets`, Git blob/commit read-back,
native outputs/negative diagnostics, and independent review. Java, LLVM,
Wasm and .NET must each execute their own compiler/guest lane against the
same source semantics; a Cranelift pass is not a substitute. A zero-step
failure remains runner admission, not a compiler failure.
