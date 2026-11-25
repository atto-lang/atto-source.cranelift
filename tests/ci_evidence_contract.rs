//! Structural checks are not CI execution receipts or guest-language conformance.
const WORKFLOW: &str = include_str!("../.github/workflows/native-cranelift.yml");
const CAPABILITIES: &str = include_str!("../atto-capabilities.json");
const REQUIRED_CASES: &str = include_str!("../tools/rollout-evidence/cases.tsv");
const PHYSICAL_SOURCE: &str = include_str!("../verification/source-integrity/verify.sh");
const PHYSICAL_TESTS: &str = include_str!("../verification/source-integrity/test.sh");

#[test]
fn workflow_enforces_exact_head_before_any_compiler_execution() {
    assert!(WORKFLOW.contains("github.event.pull_request.head.sha || github.sha"));
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("permissions:\n  contents: read"));
    assert!(WORKFLOW.contains("git rev-parse --verify HEAD"));
    assert!(WORKFLOW.contains("git write-tree"));
    assert!(WORKFLOW.contains("git cat-file -t"));
    let provenance = WORKFLOW
        .find("name: Verify commit and source tree")
        .unwrap();
    let physical_check = WORKFLOW
        .find("name: Verify physical checkout bytes, modes and path ancestry")
        .unwrap();
    let physical_negative = WORKFLOW
        .find("name: Test exact-source tamper rejections on synthetic Git fixtures")
        .unwrap();
    let receipt_tests = WORKFLOW
        .find("name: Run offline Rust receipt verifier's negative and positive tests")
        .unwrap();
    let compiler_tests = WORKFLOW
        .find("name: Compile Rust and execute native positive and negative guest tests")
        .unwrap();
    assert!(provenance < physical_check && physical_check < physical_negative);
    assert!(physical_negative < receipt_tests && receipt_tests < compiler_tests);
    for suspect in [
        "git status",
        "git diff",
        "git add",
        "git hash-object --path",
    ] {
        let preflight = &WORKFLOW[provenance..receipt_tests];
        assert!(
            !preflight.contains(suspect),
            "checkout filters in provenance preflight: {suspect}"
        );
    }
}

#[test]
fn rust_only_offline_receipt_tests_are_required() {
    assert!(WORKFLOW.contains("cargo test --manifest-path tools/rollout-evidence/Cargo.toml --offline --locked --all-targets"));
    assert!(WORKFLOW.contains(
        "cargo run --manifest-path tools/rollout-evidence/Cargo.toml --offline --locked -- --cases"
    ));
    assert!(WORKFLOW.contains("cargo test --locked --all-targets"));
    assert!(WORKFLOW.contains("cargo run --locked --quiet -- run"));
    assert!(WORKFLOW.contains("test ! -s /tmp/atto-rejected-stdout"));
    assert!(WORKFLOW.contains("grep -q 'unbound or unsupported identifier'"));
    assert!(!WORKFLOW.contains("curl | bash"));
    assert!(!WORKFLOW.contains("pip install"));
    assert_eq!(REQUIRED_CASES.lines().count(), 6);
    assert!(
        REQUIRED_CASES
            .lines()
            .all(|line| line.split('\t').count() == 3)
    );
}

#[test]
fn unexecuted_capability_inventory_remains_unverified() {
    assert!(CAPABILITIES.contains("\"repository\": \"atto-lang/atto-source.cranelift\""));
    assert!(CAPABILITIES.contains("\"certification\": \"NOT_CERTIFIED\""));
    assert!(CAPABILITIES.contains("\"guest_execution\": \"not_executed\""));
    assert!(CAPABILITIES.contains("\"actor_isolation\": \"not_executed\""));
    assert!(WORKFLOW.contains("and .certification == \"NOT_CERTIFIED\""));
    assert!(WORKFLOW.contains("and .evidence == []"));
}

#[test]
fn physical_checkout_integrity_gate_rejects_byte_laundering_before_compilation() {
    assert!(
        WORKFLOW.contains("bash verification/source-integrity/verify.sh \"$EXPECTED_HEAD_SHA\"")
    );
    assert!(WORKFLOW.contains("bash verification/source-integrity/test.sh"));
    assert!(PHYSICAL_SOURCE.contains("git hash-object --no-filters --"));
    assert!(PHYSICAL_SOURCE.contains("git ls-files --stage -z"));
    assert!(PHYSICAL_SOURCE.contains("tracked file has an unexpected hardlink"));
    assert!(PHYSICAL_SOURCE.contains("symlinked or missing tracked ancestor"));
    assert!(PHYSICAL_TESTS.contains("skip-worktree hidden content mutation"));
    assert!(PHYSICAL_TESTS.contains("clean-filter physical-byte laundering"));
    assert!(PHYSICAL_TESTS.contains("tracked directory symlink escape"));
    assert!(PHYSICAL_TESTS.contains("tracked physical hardlink escape"));
}
