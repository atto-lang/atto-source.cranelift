# Oreslang Cranelift backend

Experimental **independent** Rust/Cranelift code-generation backend for Oreslang.

This repository is **not Forge** and must never become a compiler or runtime dependency of `oreslang-source.forge`. Forge is intended to become self-hosted in Oreslang; this backend uses Rust and the upstream Cranelift code generator.

**Status:** bootstrap repository initialized; no compiler on `main` yet. Native functionality must be demonstrated by executed tests before the backend is called working. A first compiler implementation is proposed in a review branch.

Security: Cranelift JIT machine code runs inside the host process. JIT compilation alone does not provide actor isolation or an untrusted-code sandbox. Keep actor heap, mailbox, capability and supervisor acceptance separate from codegen.

Do not claim language parity or self-hosting without exact-head execution evidence.
