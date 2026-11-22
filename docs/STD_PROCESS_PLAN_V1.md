# std.process — native Cranelift compiler

This backend owns `stdlib/process_policy.atto` and its own import-based
`tests/fixtures/std-process-policy-v1/main.atto` expected-exit-42 fixture.
They contain pure guest admission checks only. Their success never grants
process creation, permission to execute a shell, or a real OS handle.

The public target is an async `process.spawn(Command)` returning an owned
`Result<ChildProcess,ProcessError>`, `wait()`, `terminate()`, `kill()`,
stdin/stdout/stderr bounded streams and a bounded `run()` convenience API.
The host must authenticate RUN, perform canonical command and environment
admission at the actual spawn boundary, and preserve actor-local permission
attenuation. Return denial, spawn failure, timeout and ordinary exit as
**distinct** outcomes. Avoid PID reuse and leaking raw host process objects.

## Cranelift native provider (not implemented)
Lower only a verified typed `Result<ChildProcess, ProcessError>` host ABI. POSIX deployments should prefer `posix_spawn` (not raw fork into a running actor process); Windows uses `CreateProcess`. Require an authenticated RUN capability scoped to a canonical executable plus explicit cwd/env/file grants and atomically reserved child/pipe quotas. Use nonblocking stream pipes, backpressure, monotonic deadlines, supervised cancellation and generation-bound handle tickets; reap exactly once. Freestanding or non-hosted compilation profiles must reject launch. A successfully executed Rust host unit test is not a guest binary receipt.

## Verification status

The `.atto` policy and fixture are source-only. The backend has not proven
parse/typecheck, linked import, real guest execution, AOT, an OS spawn,
stream/cancellation lifecycle or exact-head CI for this change. Mark each as
**NOT EXECUTED / NOT VERIFIED**, not as full `std.process` parity.
