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

## Unix source file admission

The CLI opens source descriptors nonblocking before validating that the
*opened descriptor* is a regular file. This rejects FIFO inputs, including
symlinks to FIFOs, in both `run` and `ir` without waiting for a writer.
A dedicated native CLI regression imposes a three-second hang guard.
Non-Unix systems retain the existing source-file handling behavior and do
not claim that nonblocking POSIX guarantee.
