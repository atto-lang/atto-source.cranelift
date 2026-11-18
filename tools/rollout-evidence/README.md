# Cranelift rollout-evidence inspector (opt-in Rust tool)

This dependency-free, offline Rust utility enforces a canonical receipt format and rejects malformed source/fixture evidence metadata. It **does not** authenticate CI, run any .atto guest, certify JIT/native code, or authorize private source export.

Acceptance cases in `cases.tsv` are **proposed evidence targets**, not passed tests. Each required suite must have a separately verified, actual Cranelift compiler/execution or diagnostic receipt. Synthetic unit-test receipts prove parser behavior only.

Run:
```sh
cargo fmt --manifest-path tools/rollout-evidence/Cargo.toml -- --check
cargo clippy --manifest-path tools/rollout-evidence/Cargo.toml --offline --locked --all-targets -- -D warnings
cargo test --manifest-path tools/rollout-evidence/Cargo.toml --offline --locked
cargo run --manifest-path tools/rollout-evidence/Cargo.toml --offline --locked -- --cases
cargo run --manifest-path tools/rollout-evidence/Cargo.toml --offline --locked -- receipt.tsv EXPECTED_COMMIT EXPECTED_TREE EXPECTED_CORPUS_SHA256
```

Receipt header (tab-separated, exactly one final LF for each line):
```text
repository	suite	backend	phase	outcome	commit	tree	corpus_sha256	run_id	job_id	steps	tests	exit_code	artifact_sha256
```

The 40-hex commit/tree and 64-hex corpus pins are passed independently; receipt and manifest must be canonical ASCII/LF TSV with no control bytes other than tabs and line feeds. Required records must be uniquely present, with `pass`, exit `0`, nonzero counters and artifact digest. A rejection case's exit `0` refers to its negative-test harness, never a successful compile of invalid source. This checks only metadata consistency and prints `CONSISTENT_UNAUTHENTICATED_RECEIPT`. Independent GitHub job, provenance, binary and guest execution checks remain mandatory.

No Python tests or scripts are added. Compilers and runtime behavior are unchanged.
