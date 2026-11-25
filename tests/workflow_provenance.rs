//! Source-only assertions on CI admission. Not a compiler or guest receipt.
const WORKFLOW: &str = include_str!("../.github/workflows/ci.yml");

#[test]
fn source_ci_checks_exact_pr_head_and_index_without_persisted_credentials() {
    assert!(WORKFLOW.contains("github.event.pull_request.head.sha || github.sha"));
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("git rev-parse --verify HEAD"));
    assert!(WORKFLOW.contains("git write-tree"));
    assert!(WORKFLOW.contains("git cat-file -t"));
    assert!(WORKFLOW.contains("cargo test --locked --all-targets --verbose"));
}

#[test]
fn exact_source_preflight_avoids_checkout_filter_capable_commands() {
    let preflight = WORKFLOW
        .split("name: Verify exact commit and indexed Git tree without checkout filters")
        .nth(1)
        .expect("missing exact-source preflight")
        .split("name: Verify exact client-scalar guest SHA-256 identities")
        .next()
        .expect("native source preflight must precede guest checks");
    for line in preflight.lines() {
        let s = line.trim();
        if s.starts_with('#') {
            continue;
        }
        assert!(
            !s.starts_with("git status")
                && !s.starts_with("git diff")
                && !s.starts_with("git add")
                && !s.starts_with("git hash-object --path"),
            "preflight must not invoke checkout filters: {s}"
        );
    }
}
