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

## Checked unwrap host carrier, generation fencing

This forward-port isolates the host-only subset of #143 from unrelated stale
compiler changes. Errors have private tenant, parser generation and identity
fields. A fresh parser incarnation cannot observe another incarnation's Error,
even with the same tenant ID. A foreign Error is a distinct denial, not a
catchable ordinary unwrap Error; it must not allocate an Error identity.
Checked Option None and Result Err produce ordinary envelopes; Result retains
the original Error without formatting or replacing its identity. The native
guest ABI, cleanup, transfer, and scheduler integration are still unsupported.
The real compiler must continue to reject checked unwrap source without CLIF,
JIT output or object bytes. This is not a guest parity promotion.
