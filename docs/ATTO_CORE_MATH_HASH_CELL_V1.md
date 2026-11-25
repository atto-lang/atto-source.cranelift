# Cranelift core math/hash/cell source candidate

Backend-owned Atto `.atto` source files are staged in
`library/atto.core/`; they are **not** an executable library at the
current restricted signed-i64 Cranelift stage. An ordinary Rust
integration test checks Atto source text and the portable noncryptographic
hash reference, not a JIT guest. No host Rust algorithm substitutes for
Atto runtime code.

Dependencies: compiler-owned import resolver, checked i64 and bool
function lowering, module identity/provenance, import shadow rejection,
safe cell lifetimes and compiled guest test. Existing Cranelift #82
proposes explicit fail-closed `E_LIBRARY_PROFILE_UNSUPPORTED` for all
current library imports; this feature must not bypass that safety gate.
A candidate positive source lives in
`contracts/core-math-hash-cell-v1/guest.atto`, not a passing fixture.

Hash is not cryptographic, and Cell remains **policy only** rather than
`Cell<T>` or `RefCell<T>`. Exact-head native guest and CI evidence:
**NOT_EXECUTED**. See conformance #570.

## 2026-11-22 owner/borrow parity follow-up

The Cranelift-owned `cell.atto` adds pure conservative
`valid_loan_state`, `can_owner_read`, `can_owner_write`,
`can_move_owner`, and `can_suspend_unpinned` predicates on bounded
(reader count, writer-active) state. These are **non-capabilities** and cannot
issue a reference or upgrade a loan. A Rust source-only regression covers
boundary states. There is no linker-authorized Cranelift JIT guest for this
core module, no compiled owner/loan CFG proof, and no exact-head native test
evidence yet; all remain NOT_EXECUTED.
