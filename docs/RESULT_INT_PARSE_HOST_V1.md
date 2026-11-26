# Cranelift error handling v1 staging

HOST_ONLY: src/result_int_parse_v1.rs has a bounded signed-i64 decimal
parser with tenant-local Error identity, strict ASCII digits, 1..1024
UTF-8 byte admission and immutable Ok/Err values. It is not yet linked
to a source-compiled guest type or machine ABI.

The stage-0 Cranelift frontend cannot lower Result/Option, int.parse,
postfix ?, trap or tagged match. tests/result_int_parse_admission.rs
requires exact .atto sources to reject in both IR and run modes without
emitting CLIF, JIT output, or an observable guest value, while a scalar
control still executes 42. This is compiler admission evidence only,
not semantic parity.

Next: typecheck generic tagged ADTs, lower to an authenticated owned
native guest ABI, retain the original Err on ? and execute full
conformance on exact source/tree/toolchain. Never promote source-only
or host-only tests as guest pass. Follow spec #124, conformance #68,
parity tracker #20.

## Checked unwrap host contract (2026-11-26)

The host-only carrier now distinguishes ordinary OptionNone and ResultErr
unwrap failures from foreign Error rejection. A fresh private parser
generation plus tenant and monotonic identity fences repeated tenant IDs;
wrapping an owned Err retains its original Error value and identity and
mints a separate envelope. This is not an authenticated guest-visible
native handle, public subtype hierarchy or linked Cranelift lowering.

Four unwrap source cases must reject in check, run, ir, emit-object and
emit-object-strict modes with no emitted artifact. These are negative
admission controls, not positive semantics. The new Rust host test and
compiler tests are AUTHORED_NOT_EXECUTED: a local Rust toolchain is absent.
Full typed ADT representation, ordinary error status transport, cleanup,
JIT/object execution and independent review remain required.
