# Shared checked-i64 corpus via real native Cranelift JIT

**Scope: native single-entry checked-i64 adapter; not full source-language parity.** This adapter builds actual native Cranelift JIT code, rather than evaluating arithmetic with Rust host helpers.

The case file `contracts/checked-i64-all-backends-v1/cases.tsv` has exact Git blob `71309fac2ab6b2191a0056c84d78987bd342dce0` and is byte-identical to Java [#66](https://github.com/atto-lang/atto-source.java/pull/66), JavaScript [#69](https://github.com/atto-lang/atto-source.js/pull/69), and the central 11-lane [conformance #235](https://github.com/atto-lang/atto-platform-conformance/pull/235). It remains the same seven values, seven runtime overflow, one zero divisor.

## Important language subset distinction

Current stage-0 Cranelift accepts one scalar `main` function with at most four signed-i64 source parameters. It cannot yet compile the source-level named `sum`, `subtract`, `multiply`, `opposite`, `remainder` helper declarations that Java/JS use. The new `tests/checked_i64_shared_corpus.rs` **checks the exact original expression for each reviewed case** then rewrites it into a semantically equivalent *parameterized main* source expression. This keeps runtime overflow dynamic and tests that the real native JIT, not host constant evaluation, detects intermediate overflow. It is an explicitly bounded **adapter profile** — **not** full unchanged multi-function source parity.

The test independently verifies the exact corpus Git blob by `git hash-object`, creates source files with `create_new` and Unix mode 0600 (no symlink clobber), runs `orescranelift ir` to check real CLIF generation, and runs `orescranelift run` with checked signed inputs. It requires exact numerical stdout on successes and compiler-owned error messages, nonzero diagnostic-only exit and no stdout on overflow/zero denominator. All original 15 entries must be exercised without silent skipping.

## Run on a real compiler host

```sh
cargo test --locked --test checked_i64_shared_corpus -- --nocapture
cargo test --locked --test named_integer_contract_guest
cargo test --locked --test parity_signed64
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

The existing `.github/workflows/native-cranelift.yml` and `.github/workflows/ci.yml` both run `cargo test --locked --all-targets`, so this new test is integrated into the existing CI source tree, not a duplicate workflow.

**Execution evidence:** [alternate-runner CI 45547035646](https://github.com/22-factor-apps/atto-ci/actions/runs/45547035646) on source commit `1c074512807cf66c869b8c287ebc9f97e9135447` verified the exact corpus blob, all 15 adapted native JIT cases, the changed-expression negative mutation, existing scalar regression tests, and strict Clippy. The only failing gate was Rustfmt; its exact output was applied afterward. Final exact-head checks must pass before merge. No full unchanged multi-function guest parity is claimed.

**Further scope:** add production source function-declaration/call lowering and general module graphs only with type/ownership proofs. Then compile the exact unchanged shared Java/JS function-bearing `.atto` guest source, rather than this adapter, to promote true source-level parity. Do not treat this trusted JIT as a secure untrusted TenantVM sandbox, full AOT, hard-real-time implementation or actor runtime.
