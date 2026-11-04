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
grouping, unary +/-, binary + - *, signed remainder with a proven nonzero
literal divisor (`%`), and line comments.
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

Signed remainder follows Java truncation-toward-zero semantics, including
`i64::MIN % -1 == 0`. This intentionally matches the restricted LLVM
literal-divisor parity slice, not general Java runtime expression coverage.
The numerator still executes checked arithmetic before computing remainder.

## Explicit security boundary

Native JIT code executes inside this process. **It is not a sandbox** and
there is no guest actor heap isolation or untrusted-source support.
The entry ABI is internal (four i64 operands plus an output pointer returning
an i32 status), not a published AttoLang or foreign-call ABI.

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
