# Cranelift stage-0 source provenance and evidence boundary

- **Canonical target:** `atto-lang/atto-source.cranelift`
- **Original implementation:** `ores-truffle-oreslang/oreslang-source.cranelift`
- **Pinned original commit:** `d4acaf1dc36117b09d6a765665c17d55bea6c18b`
- **Review sequence:** #4 (17 compiler/toolchain files), #5 (10 standalone native Rust integration tests), #6 (27 scalar parity tests/manifests/fixtures), #7 (CI and explicit capability inventory transition).
- All 54 imported original files were recreated as Git blobs in the canonical repo with exact matching source/destination Git SHA-1 values. These verify content identity, **not** passing compiler or guest tests.
- Canonical repository language and parity policy is governed by `atto-spec`, `atto-platform-conformance`, `atto-compilers-parity-testing`, and `atto-target-profiles`. Historical `.ores` fixtures retain exact bytes; new portable source should use `.atto`.
- `source_state = implementation` means Rust implementation code exists, not that it compiles, that a guest ran, or that release support is certified. All feature statuses remain `not_executed` and certification remains `NOT_CERTIFIED` until independently verified.
- GitHub Actions results with no allocated runner steps, cancelled jobs, or zero-step failures are admission failures. They must not be counted as successful guest executions.
- **Minimum narrow receipt:** exact source head/tree SHA, `cargo test --locked --all-targets` success, actual CLIF native guest execution with positive and negative controls, target platform and Rust compiler version.
- **Forbidden extrapolation:** actor isolation, ownership bang, hard-real-time, full AOT, and cross-backend parity cannot be inferred from scalar-only tests.
- **Scripts:** no Python implementation, prototype or Python test harness is permitted in the canonical repository.
