# HTTP actor v1 admission boundary

Three byte-identical canonical fixtures from atto-platform-conformance
contracts/http-actor-concurrency-lifecycle-v1.json are included in
tests/fixtures/http-actor-v1. The first two are positive declaration cases
on a backend with actual HTTP actor lowering. On the current scalar Cranelift
backend all three must fail closed; no silent scalar fallback or native HTTP
claim is permitted. The 3x4 case additionally requires explicit nonserial
rejection once the serial actor declaration is implemented.

Host inventory: node --test tests/http-actor-fixture-pins.test.mjs
Compiler-negative gate: cargo test --locked --test http_actor_unsupported_gate

The inventory is not compiler execution. A compiler-negative result is not
native HTTP execution or full parity. Replace the negative expectation for
the 1x1 cases only when real transport actor lowering is implemented and
validated against the common lifecycle contract.
