# Cranelift-owned Atto core memory/atomic policy sources

Freestanding candidate `.atto` in `stdlib/core/mem.atto`, `ptr.atto`,
`atomic.atto`. These are backend-local: no forced cross-compiler source
identity or host Rust stdlib implementation.

`core/mem` checks align-up, stride and bounded byte sizes;
`core/ptr` checks region-relative span/index offsets and alignment;
`core/atomic` validates memory orders 0 relaxed, 1 acquire, 2 release,
3 acq_rel and 4 seq_cst including CAS failure restrictions. No actual
pointers, shared memory, Atomic<T>, CAS, RMW, CPU fence, GC, allocator
or actor-lifetime privileges are granted. Invalid arithmetic returns -1
or false, and unsupported memory operations remain fail-closed.

The 63 cases in `tests/core-memory-policy-source.test.mjs` are
restricted source-only scalar tests, **NOT** Cranelift compiled guest,
native AOT or memory-order evidence. Import resolution and real native
negative/positive guest fixtures require separate compiler work, exact-head
runner results and independent security review.

Coordinate Java #1771, LLVM #873 (source policies) and LLVM #801
(actual Atomic/Box AOT), Rust #537 and #419, and atomic spec #175.
