# AttoLang Cranelift backend

**AttoLang** is the current public language name (formerly **Oreslang**).
This branding change does not imply full-language parity, AOT or zero-GC guarantees.
The repository's existing scalar `.atto` and `.ores` integration fixtures are retained.
These files and naming changes do not certify this draft's new remainder or CI hardening.
Do not rename repository identifiers or fixed ABIs solely for public branding.

An **independent Rust + Cranelift** experimental backend for AttoLang. It is
**not** part of `oreslang-source.forge`. Forge remains on its own self-hosting
path, and must never link to Cranelift.

## Stage-0 executable scalar profile (`.atto` and legacy `.ores`)

This backend compiles a deliberately narrow AttoLang source subset
from `.atto` or legacy `.ores` files into **actual Cranelift native JIT machine code**, with a checked overflow
status rather than an unchecked wrap or process trap.

```oreslang
fnc main(x:int,y:int):int{return (x+2)*y;}
```

```sh
cargo run --locked -- run example.atto 7 8 # 72
cargo run --locked -- ir example.atto      # generated Cranelift IR
cargo test --locked --all-targets
```

Supported: one `main` function, 0–4 typed `int` parameters (`int x` or `x:int`),
signed i64 literals (including MIN and digit separators such as `1_000`),
grouping, unary +/-, binary + - *, named `Integer.rem(numerator, divisor)` with a
compiler-proven nonzero signed literal divisor, and line comments.
The first additional Java-defined expression is `Integer.bitXor(a,b)` on signed64
values. Its arguments evaluate left-to-right and the compiler emits Cranelift's
native `bxor` instruction; caret operators remain rejected. Other `Integer.*`
named functions (`mod`, `pow`, dynamic-divisor `rem`) remain unsupported.
Both return syntax variants `:int` and `->int` are accepted. Precedence and
arithmetic overflow match the checked-i64 scalar contract, not floating
point division. Source budget 1 MiB; AST 4096 nodes; nesting 128.

Rejected: **everything else**, including imports, helper functions, user-defined calls,
classes, actors, heaps, ownership, recursion, async, control flow, division, dynamic/zero-divisor remainder,
structural types, UTF-8 identifiers, and any trailing code.

`%` syntax is retired and rejected at lexical admission. The restricted named
`Integer.rem` lowers to actual native signed remainder; dynamic and zero
divisors are still rejected before JIT rather than pretending full Java parity.
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
establish full AttoLang support, actor isolation or Forge self-hosting.

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

On Unix, the CLI opens source descriptors with `O_NONBLOCK`,
`O_CLOEXEC`, and `O_NOFOLLOW` before validating that the opened descriptor
is a regular file. This rejects FIFOs without blocking, and rejects a final
source-path symlink even when it points to an otherwise valid regular source.
Native CLI regressions cover both `run` and `ir`, including a three-second
hang guard. This check does not reject symlinked **parent directories**, and
is not a sandbox against a malicious compiler or concurrent file mutation.
Non-Unix systems retain the existing source-file handling behavior and do
not claim POSIX no-follow guarantees.
