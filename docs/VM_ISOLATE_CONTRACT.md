# OresVM / IsolateVM contract v0 (backend admission)

**Status: experimental host substrate.** Not full actor VM execution,
complete cross-backend parity, per-tenant physical resource enforcement or
a security sandbox. No general guest-code tenant isolation is claimed.

## Common vocabulary

- **OresVM:** root host, code registry, permission broker, global admission
  and tenant supervisor. It must not own tenant-writable guest module globals.
- **IsolateVM:** an OresVM-owned execution domain for one tenant incarnation;
  owns private mutable globals, heap/resource limits, actor registry and
  quiescence accounting. Reusing a tenant ID creates a new generation.
- **Actor:** individually scheduled concurrent unit inside exactly one
  tenant's IsolateVM, with sealed state, owned arena and mailbox.
- **Isolate execution context:** a versioned *borrowed entry capability*;
  never a raw writable pointer to the OresVM root or another tenant.

## Required lifecycle invariants

1. Created -> Running -> Quiescing -> Stopped -> destroyed.
2. Reject duplicate tenant IDs, unbounded admission, invalid quotas,
   stale/foreign/forged handles and actor ID replay after reincarnation.
3. Closing admission during quiescence must be atomic with new guest,
   actor, mailbox, callback and I/O registration accounting.
4. Complete shutdown only after guest calls and all actor,
   continuation, callback, mailbox-loan and I/O retirement have drained.
   Accounting is insufficient without physical OS/worker retirement.
5. Cross-isolate messages must be versioned and serialized or explicitly
   transferred by checked ownership. No borrowed raw pointers cross tenants.
6. A shared immutable AOT module may have different *tenant-owned*
   writable globals. Neither mutable module state nor user symbol identities
   silently become process global.
7. Include a distinct status/result ABI, including legitimate signed
   negative values. Check native artifact entry ABI **before** function
   pointer casts; ABI version mismatch must fail closed.
8. An in-process isolate is **not an OS security sandbox**. Native/FFI
   arbitrary pointers require process-level separation for hostile tenants.

## V8 lessons and crucial differences

V8 separates an isolate's engine/heap from its Context globals and supplies
host platform scheduling and explicit entry/handle scopes. Oreslang should
borrow explicit context ownership, generation-fenced handles, snapshots
for immutable initialization and fast tenant-specific state, **not**
the assumption that a V8 Isolate is equivalent to a renderer-process
security boundary. Oreslang must eventually schedule independently sealed
actors within one tenant concurrently, rather than imposing one
entering thread for the whole tenant. Pointer compression changes memory
representation, not authorization. In-process host admission is insufficient
for arbitrary untrusted native machine code.

Architecture reference: LLVM draft PR #1158,
`docs/ORESVM_VS_ISOLATEVM.md` in `oreslang-source.llvm`.

## Cranelift Rust implementation

`src/isolate.rs` implements OresVM host registry, strongly owned
generation-fenced IsolateHandle/ActorTicket identities and explicit
resource-pin retirement gates in pure Rust. It is compiled as a module
of the existing Rust/Cranelift binary and has a unit test. It is not
connected to native Cranelift JIT entrypoints yet: code executed through
the current in-process JIT still has host-process memory access and is
**not** untrusted multi-tenant sandboxing. Never link Forge to Cranelift:
these backends remain independent. The next implementation must pass a
versioned isolate context to Cranelift-generated guest code and then
define an authenticated out-of-process executor for hostile guests.
`cargo test --locked --all-targets` must be executed in a real runner;
Rust 1.99.1 was unavailable in the local container.

## First compiled machine-code tenant execution (new experimental slice)

The Cranelift backend now exposes `Compiled::run_for_isolate(&OresVM,
&IsolateHandle, delta)` for exactly one trusted two-argument signed-i64
function. The function is **compiled once to real native Cranelift JIT code**;
the OresVM host loads each tenant's private scalar state, calls the same
compiled function with `(old, delta)`, and writes the result into that tenant
only when the native call succeeds. Checked arithmetic errors leave tenant
state unchanged; negative results remain valid.

`OresVM::invoke_trusted_step` pins a queued/running guest call, serializes
*within a tenant* (not across tenants), rechecks admission after waiting,
and prevents shutdown while any native call remains active. Quiescence
denies new work; already admitted and running calls may finish.
The integration tests execute the actual compiled Cranelift code for
two tenants, assert independent state, overflow nonmutation, reincarnation
and active-call shutdown fencing.

**Security limitation:** the compiled function's existing ABI is still
four i64 operands + result pointer, returning status; the OresVM host
mediates tenant state rather than exposing a raw IsolateVM pointer to JIT
code. This is **not** a native sandbox, a general actor scheduler, or a
versioned foreign IsolateContext ABI. Guest code is trusted in-process
and must not retain runtime pointers. Untrusted native code requires
a supervised OS worker. Real cargo tests remain mandatory before merge.
