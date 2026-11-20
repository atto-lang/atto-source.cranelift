# ADT source admission and six-backend parity boundary

**Target contract:** `oreslang-platform-conformance` #270 (36 `.ores`
cases), with the additive six-backend v2 inventory being developed.

Cranelift stage-0 compiles **scalar i64 source only**. Its nominal ADT
capability fence returns an explicit `DECL_KIND_UNSUPPORTED: enum` error on `define enum`, rather than attempting a scalar
JIT or emitting partial CLIF. Type-only `define pattern` remains unsupported.
Real `Option`/`Result` constructor pattern matching requires typed
lowering and should not be mistaken for a generic parser refusal.

The eight `tests/fixtures/adt-v1/*.ores` sources were copied byte-for-byte
inside the same organization from the canonical conformance inventory.
Their Git blob SHA-1 identities are pinned below.

| Fixture | Git blob |
| --- | --- |
| nullary | a59622d03e0c2d66a28c08d082140b769f747499 |
| payload | ece5ad07cfc0802907f0fc18a306b8f726d3f169 |
| owner | 8ea4459d0e89adce0236fef0daa29833bb5a2e87 |
| max-tag | b0d58c31c25afd871f739ed8538f0e17196aaa3e |
| guarded-not-exhaustive | 80795a49084538e1c4e3ba79197edcff92ddf2aa |
| direct-unboxed-recursion | 5c0179babb71ab3ccc4ee60c7141e13e90ede4f2 |
| option-result-partitions | 71815d126707ab13ed54b9dde4ba2645c7c8472b |
| option-partial-int | 9af4f4c1bdd1fa8547ea86607b386415a6c6bf2e |

The integration tests invoke **both** `run` and `ir`, require exit code
1 with compiler stderr and **no stdout** for all eight candidate ADT
sources, and retain a positive real scalar JIT/CLIF control. This
tests fail-closed *admission*, not the expected positive/negative
language semantics. Full parity requires nominal type ownership,
constructor tagging, finite-domain exhaustiveness, aligned payload
layout, borrow/drop safety, actual Cranelift guest execution and
nonzero-step authenticated CI with exact compiler/tree provenance.

Neither this backend nor `oreslang-source.forge` should be credited
for unimplemented ADT behavior or for a zero-step workflow.
