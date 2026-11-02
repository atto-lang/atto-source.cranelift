//! Source-free CI admission assertions. Does not execute an AttoLang guest.
const WORKFLOW: &str = include_str!("../.github/workflows/ci.yml");

#[test]
fn native_ci_checkout_is_bound_to_pr_head_without_persisted_credentials() {
    assert!(WORKFLOW.contains("github.event.pull_request.head.sha || github.sha"));
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("test \"$(git rev-parse --verify HEAD)\" = \"$EXPECTED\""));
    assert!(WORKFLOW.contains("test \"$(git write-tree)\" = \"$SOURCE_TREE\""));
    assert!(WORKFLOW.contains("git cat-file -t \"$SOURCE_TREE\""));
    assert!(WORKFLOW.contains("cargo test --locked --all-targets --verbose"));
}

#[test]
fn preflight_does_not_invoke_git_worktree_clean_filters() {
    let preflight = WORKFLOW
        .split("name: Verify exact commit and indexed Git tree without checkout filters")
        .nth(1)
        .expect("exact-source CI preflight must exist")
        .split("uses: dtolnay/rust-toolchain@")
        .next()
        .expect("preflight must precede Rust toolchain setup");
    for line in preflight.lines() {
        let command = line.trim();
        if command.starts_with('#') {
            continue;
        }
        assert!(
            !command.starts_with("git diff")
                && !command.starts_with("git status")
                && !command.starts_with("git add")
                && !command.starts_with("git hash-object --path"),
            "untrusted checkout filter-capable Git command in preflight: {command}"
        );
    }
}
