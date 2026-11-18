# AttoLang Cranelift backend

## AttoLang migration and authoritative language contracts (2026-11-18)

**AttoLang** is the current language name (**formerly Oreslang**). This `ores-truffle-oreslang/oreslang-source.cranelift` repository retains the historical name and implementation context; the AttoLang-named counterpart is **[atto-lang/atto-source.cranelift](https://github.com/atto-lang/atto-source.cranelift)**, the destination for new AttoLang development. This legacy Cranelift tree and the AttoLang Cranelift repository may differ in code and tests. Maintain independent proof for its compiler and runtime.

The **[atto-spec](https://github.com/atto-lang/atto-spec)** repository now owns normative language syntax and semantics, rather than this backend README, the old Oreslang repository names, or Java/Truffle behavior alone. Portable, positive **and negative** language tests belong in [atto-platform-conformance](https://github.com/atto-lang/atto-platform-conformance); compare implementations via [atto-compilers-parity-testing](https://github.com/atto-lang/atto-compilers-parity-testing). [atto-common-source](https://github.com/atto-lang/atto-common-source) owns portable `.atto` libraries. Backend-specific compiler code stays in its corresponding `atto-source.*` repository.

**Current source-design directions (targets, not claims this backend implements them):** canonical `.atto` for new portable source, `snake_case` function/method names, ownership operations `borrow!`, `take!`, `share!`, `copy!`, and `copy!!`; [implicit borrowing for nonprimitive arguments](https://github.com/atto-lang/atto-spec/blob/main/language/implicit-argument-borrow-v1.md) is still a design target subject to compiler verification. [File/module `define const`](https://github.com/atto-lang/atto-spec/blob/main/language/file-module-const-v1.md) supports proposed constexpr values **and** non-throwing, one-time runtime `os.read_env` initialization. [The proposed operator contract](https://github.com/atto-lang/atto-spec/blob/main/language/bitwise-and-stream-operators-v1.md) uses `<<<`, `>>>`, and `ashr` for shifts and reserves `<<`/`>>` for directional stream I/O; it is **not** automatically supported by legacy backends. [Shared ownership](https://github.com/atto-lang/atto-spec/blob/main/language/shared-ownership-and-guards-v1.md) does not grant unsynchronized mutable aliasing. Check spec documents for proposal/admission status before changing a parser or example.

**Compatibility and verification:** Keep existing `.ores` fixtures, `oreslang-*` commands, package IDs, fixed ABIs, and wire identifiers wherever still used; an AttoLang name change alone does not prove that a renamed CLI, syntax form, AOT target, actor isolation, zero-GC runtime or guest feature works. Retain exact legacy commands below when they describe this checkout. Do not use Python for project scripts, tests, CI helpers or tooling; use Rust for supporting tooling/tests where practical, without misrepresenting the backend implementation language. Record source commit/tree, target/toolchain, real positive/negative **compiled guest execution**, and `passed`/`failed`/`unsupported`/`not executed` for every claimed parity result. Host-only tests or workflows with zero executed steps are **not** compiler parity evidence.

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
The named signed64 bitwise expressions `Integer.bitXor(a,b)`, `Integer.bitAnd(a,b)`
and `Integer.bitOr(a,b)` compile through actual Cranelift `bxor`, `band` and `bor`
instructions, respectively. Both operands are evaluated left-to-right with checked
arithmetic; unsupported `&`, `|`, `^` and `^^` source operators remain rejected.
These bitAnd/bitOr names are a **Cranelift candidate extension** pending shared
language-contract and cross-backend review, not language-wide parity.
Named `Integer.rem(a,b)`, `Integer.mod(a,b)`, and `Integer.pow(a,b)` also lower to native checked i64 code. Remainder follows truncation toward zero; modulus is nonnegative for either divisor sign. Zero divisors and negative exponents reject at execution. Power uses checked exponentiation by squaring.
Both return syntax variants `:int` and `->int` are accepted. Precedence and
arithmetic overflow match the checked-i64 scalar contract, not floating
point division. Source budget 1 MiB; AST 4096 nodes; nesting 128.
The 128-level bound applies to the constructed AST, including left-associative
operator chains, not only parser recursion. Over-depth input is denied before CLIF.

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
all runtime arguments. Numeric guest arguments are checked before source
file access; invalid Unicode OS argument encodings reject deterministically
rather than panicking. The source filename remains a platform-native path.
This argument-vector admission is not a resource quota on the host process
and does not authorize hostile source execution.

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

## Legacy repository scope and contribution routing (2026-11-18)

**Historical checkout:** `ores-truffle-oreslang/oreslang-source.cranelift`. **AttoLang destination:** [`atto-lang/atto-source.cranelift`](https://github.com/atto-lang/atto-source.cranelift).

**Scope:** Independent Cranelift code generation and native guest execution experiments. The historical repository and new counterpart may not contain identical commits, pull requests, issues, or artifacts; a same-suffix repo is not proof of an in-place GitHub ownership transfer or identical Git history. Compare exact SHAs and migrate outstanding work deliberately; open new AttoLang design changes against [atto-spec](https://github.com/atto-lang/atto-spec) and new backend work in the appropriate AttoLang repository. The former plan to transfer `oreslang-source.cranelift` under its unchanged legacy name is superseded as a naming destination by `atto-source.cranelift` (without implying the old GitHub repository itself has moved).

Keep the [historical organization migration tracker](https://github.com/ores-truffle-oreslang/.github/issues/26) and [review runbook](https://github.com/ores-truffle-oreslang/.github/pull/25) for migration/security context, not as proof of a completed transfer. Preserve access controls, private source provenance, branch protections, PR/issue history and CI evidence; do not assume an existing `atto-*` repository is a verified byte-for-byte transfer. Host-only or source-free canaries do not prove guest code execution or parity.

## Canonical browser integration and native actor parity gap

The browser Wasm/ESM compiler is staged in the still-open `oreslang-source.rs` [PR #52](https://github.com/ores-truffle-oreslang/oreslang-source.rs/pull/52), not yet merged into `main`. Its intended checked-IR emission and downstream source.js/source.wasm artifact consumption remain separate verification obligations. This independent native Cranelift compiler is a separate backend and does not become browser-capable by renaming native machine code to Wasm.

`tests/canonical-browser/actor-state.atto` has the same Git blob SHA (`41a5abf398bb025639c5e190ee1c93e569a9b035`) as `browser/tests/fixtures/pass/61-actor-state.atto` at source.rs PR #52 head `9b40edd07e78c69f541408ad41d7349f34725f1c`; this establishes fixture identity, not executed browser parity. The regression invokes this backend's actual `run` and `ir` commands and requires an explicit unsupported-actor diagnostic with no output. This records the gap; it does not count as successful actor guest execution. Native nominal actors, ActorMail receive, owned linear-memory domains, mailbox scheduling and supervision must be lowered before the same source can pass here. Browser Wasm/ESM evidence cannot certify this backend.
