# Cranelift private-source CI admission (review proposal)

**Status: REVIEW ONLY / NOT AUTHORIZED.** Neither a GitHub connector Git-tree copy nor a job with no cross-organization checkout token removes the private-source disclosure boundary. This document and CODEOWNERS do **not** authorize source export, increase GitHub Actions minutes, activate branch protection, or attest native compilation.

## Independent administrative prerequisites

1. **Source authority** — Require the private source repository's default `main` to be effectively protected: PR-only changes, independent CODEOWNER approval after last push, stale-review dismissal, required non-spoofable checks and conversation resolution; prevent force push/deletion and administrator bypass. Confirm actual effective rulesets, not merely the presence of CODEOWNERS.
2. **Destination authority** — Approve a single private destination **by immutable GitHub repository ID**, with documented reader principals, audit/retention policy, restricted runner group and workflow, effective protected `main`, and independent destination maintainer/security signoff. Its private visibility alone is insufficient. Confirm actual runner execution with nonzero steps; successful historical capacity does not establish live billing allowance.
3. **Source scope** — An eligible independent source reviewer approves the one immutable source PR commit and **Git root tree**, precise allowed files/subtree, destination ID, purpose and expiry. Review identities must satisfy any denylist and may not be the PR author or unauthorized automation.
4. **Reviewed publisher** — Land a fail-closed, source-controlled publisher and explicit narrow allowlist under effective source branch protections. The publisher rechecks live source/default-branch authority, immutable PR head/base/repository ID, exact tree and destination ID before publication, before every write, and before final authorization. No broad `contents:write` on untrusted PR workflows; no token/log leakage. **Direct connector create_blob/create_tree/update_ref mirroring is not a substitute for this admission.**
5. **Carrier admission** — Run an approved exact-tree verifier on a unique isolated destination ref, with checkout confined to the carrier's own repository. Reject unexpected extra source paths, symbolic links, moved refs, dirty checkout, arbitrary workflow overrides, and changes to checked files. Record both source and mirrored subtree tree SHAs. Do not allow an untrusted PR to select a privileged carrier or override the trusted plan.
6. **Execution and evidence** — On the admitted exact tree, require `cargo fmt --all -- --check`, `cargo test --locked --all-targets --verbose`, `cargo clippy --locked --all-targets -- -D warnings`, and an actual Cranelift generated-code CLI smoke. Include all 26 strict signed64 guest vectors, overflow/zero/negative-exponent errors, FIFO/symlink source-admission regressions, and grammar negatives `%`, `^`, `^^` where applicable. Publish verified original head/tree, carrier ref/tree, run/job IDs, run attempt, actual nonzero steps and test counts. Do not mistake a host-only oracle for generated-code execution.
7. **Post-run and revocation** — Verify the final source and destination refs/tree remain unchanged, preserve a redacted evidence receipt, remove temporary refs/artifacts subject to retention rules, and revoke the narrow allowlist independently. Git object garbage collection is not guaranteed by deleting a branch.

## Blocker and evidence taxonomy

- `NOT_EXECUTED`: queued/skipped/zero-step runner or plan/billing admission failure. **No compiler PASS.**
- `SOURCE_FREE_PASS`: source-free runner capacity or security-control tests with executed steps. **Not native compiler evidence; not authorization to copy private source.**
- `HOST_ORACLE_PASS`: independent arithmetic/reference evaluation only. **Not Cranelift native execution.**
- `EXACT_TREE_NATIVE_PASS`: actual compiled guest tests on the documented immutable approved tree, with enough job/step/runner provenance to confirm execution. **Does not substitute for independent review or protected-main administration.**
- `MERGE_ELIGIBLE`: independent reviews and effective security gates satisfied, exact-tree native and policy checks valid, expected head unchanged. Merge using the expected SHA and verify resulting `main` tree plus post-merge CI.

## Current Cranelift work and prerequisites

- [Signed64 PR #6](https://github.com/ores-truffle-oreslang/oreslang-source.cranelift/pull/6) has a past authentic native run on an older tree. Its restacked head and inherited PR template require separate qualification; no approval has been submitted.
- [Legacy operators PR #8](https://github.com/ores-truffle-oreslang/oreslang-source.cranelift/pull/8) is stacked, draft and requires its **own** exact-tree native execution and independent review.
- [Candidate private carrier governance](https://github.com/honeypot-r-us-test/hnpt-lib-core/issues/15) remains a separate independent destination approval; previously observed unprotected `main` is a blocker.
- Do not privately mirror into a public test repository, use another organization's minutes by bypassing source policy, infer capacity from a past successful job, or mark unexecuted source jobs green.

**Administrator action:** Apply and verify source/destination protections and independent reviews. Until then, only source-local and source-free tests are permitted. This proposal has no privileged workflow or publisher and deliberately grants no export capability.
