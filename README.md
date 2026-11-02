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
an i32 status), not a published Oreslang or foreign-call ABI.

Direct Cranelift crates are pinned to 0.137.0, and transitive dependencies
are pinned by the committed `Cargo.lock`, generated with Rust 1.99.1. The
`rust-toolchain.toml` pin and `--locked` CI prevent silent dependency drift.

This branch remains experimental. Passing scalar JIT and native AOT guest tests
**does not** establish full AttoLang support, macro expansion, actor isolation,
or Forge self-hosting.

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

## Experimental program-specific AOT object mode (not yet a standalone executable)

`cargo run --locked -- object example.ores output.o` emits a native relocatable
object containing actual machine code generated from the admitted stage-0 scalar
AttoLang source, exporting `oreslang_scalar_entry` with the same checked i64
ABI as the JIT path: four signed i64 values, `int64_t *result`, `int32_t` status
(0 success; 1 overflow). An external host entrypoint must link this object.
The AOT code-generation path does **not** instantiate `JITModule`.

This deliberately narrow AOT progress does not prove complete AttoLang guest
AOT, actor/async/heap semantics, cross-platform standalone binaries, or
type-safe imports/macros. `define macro` and `name!(...)` are still unsupported
and must reject **before object emission**, not silently fall back to native JIT.
Future compiler phases must expand hygienic typed macros at build time before
this or any other backend emits a runnable artifact. The regular `run` command
continues to select optional trusted scalar JIT, and it is not a sandbox.

The native-object integration test links the generated object using a tiny
independent C host ABI shim and executes the result, including a checked
overflow negative case. That is native **guest-AOT object** evidence, not a
self-contained runtime binary.

## Experimental scalar standalone native guest AOT executable

`cargo run --locked -- build example.ores ./example` now emits a directly
executable native guest application on supported Unix build hosts. Run
`./example 7 8` for the source example above (72).

The pipeline parses admitted AttoLang scalar source, emits a program-specific
Cranelift native object, then links it with the compiler-owned fixed
`src/aot_entry.rs` scalar process entrypoint using build-time `rustc` and the
system native linker. The executable uses **neither runtime JIT nor a guest
interpreter**. The entrypoint supplies checked i64 argv parsing, an
exact-length argument check, a zero-filled fixed four-i64 native call ABI,
and overflow/error exit codes. The output is published atomically by a
no-overwrite hard link from a 0700 temporary build directory created in its
destination directory; rejected source and failed builds must leave no output.

This is still **only stage-0 scalar standalone guest AOT** on the host
architecture, not complete AttoLang application AOT. Macros, actor turns,
heaps, async, structured ownership and complete generics are unsupported,
and must reject without falling back to the JIT. Cross-platform executable
formats and native runtime packaging have not been qualified. Compilation
requires the pinned Rust toolchain and a system linker **on the build host**;
the linked guest program does not require those tools to execute.
