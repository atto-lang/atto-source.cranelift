# Cranelift std/time admission boundary

The experimental Cranelift backend currently supports a bounded signed-i64
scalar parser and native JIT, not the class/Result/Timer API in its independently
owned `stdlib/std/time.atto`. Importing the module must fail closed until
class linking, checked duration arithmetic, scheduler-owned monotonic clock,
actor suspension and cancellation are lowered and tested.

`tests/std_time_frontend.rs` directly includes the production
`src/frontend.rs` parser and tests two negative std/time paths plus a
positive existing scalar control. It can be compiled alone with
`rustc --edition=2021 --test tests/std_time_frontend.rs -o /tmp/time-frontend-test`
and executed without adding a fake time parser. The standalone test also
executes embedded frontend tests under `#[cfg(test)]`.

An admission rejection is evidence of a correct unsupported boundary,
**not** positive time guest conformance.
