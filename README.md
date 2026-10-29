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
grouping, unary +/-, binary + - *, signed remainder with a proven nonzero
literal divisor (`%`), and line comments.
Java-defined named signed64 source expressions `Integer.bitXor(a,b)`,
`Integer.rem(a,b)`, `Integer.mod(a,b)`, and `Integer.pow(a,b)` are supported
for typed `int` operands. Each operand is evaluated once in lexical order.
Cranelift emits native bitwise XOR, guarded signed remainder, nonnegative
Euclidean modulus, and checked exponentiation-by-squaring, respectively.
These are source-level compiler paths, not host reference-only helpers.
Divisor zero, checked multiply overflow, and negative exponents map to distinct
runtime error statuses. Legacy caret operators remain rejected; stage-0
literal-divisor `%` remains temporarily for migration and does not support
runtime variable divisors.
Both return syntax variants `:int` and `->int` are accepted. Precedence and
arithmetic overflow match the checked-i64 scalar contract, not floating
point division. Source budget 1 MiB; AST 4096 nodes; nesting 128.

Rejected: **everything else**, including imports, helper functions, user-defined calls,
classes, actors, heaps, ownership, recursion, async, control flow, division, dynamic `%` remainder,
structural types, UTF-8 identifiers, and any trailing code.

Signed remainder follows Java truncation-toward-zero semantics, including
`i64::MIN % -1 == 0`. This intentionally matches the restricted LLVM
literal-divisor parity slice, not general Java runtime expression coverage.
The numerator still executes checked arithmetic before computing remainder.

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
- Verify the **26 signed64 rem/mod/pow/bit_xor cases** from Java draft #2193
  (pinned source fixture blob `3f65f557f06e8cd0d34a738a0d357d6c2e477665`)
  through actual generated native guest code, not host-only reference logic.
  Tests are authored but **not execution-verified** for this PR head.
- Require malformed/unsupported syntax to fail without stdout.
- Record exact source commit/tree, compiler versions, and executed CI steps.
- Cross-check source results against LLVM and Forge on their **actual** common
  scalar subset, never assume parity from shared fixture names.
- Keep the PR draft until CI actually allocates a runner and executes tests.

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
