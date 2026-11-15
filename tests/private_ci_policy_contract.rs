//! Structural tests of the private-source CI deny-by-default policy.
//! A passing test never proves GitHub runner admission or authorizes export.
const WORKFLOW: &str = include_str!("../.github/workflows/private-ci-policy.yml");
const ALLOWLIST: &str = include_str!("../.github/private-ci-approved-repositories.txt");

#[test]
fn source_transfer_allowlist_has_no_active_entry() {
    for line in ALLOWLIST.lines() {
        let trimmed = line.trim();
        assert!(
            trimmed.is_empty() || trimmed.starts_with('#'),
            "private-source export destination is not authorized"
        );
    }
}

#[test]
fn deny_default_guard_is_unbroken_and_tested() {
    let production = "if grep -Evq '^[[:space:]]*(#.*)?$' \"$file\"; then";
    let synthetic_allow = "if grep -Evq '^[[:space:]]*(#.*)?$' \"$fixture\"; then";
    let synthetic_deny = "if ! grep -Evq '^[[:space:]]*(#.*)?$' \"$fixture\"; then";
    assert_eq!(WORKFLOW.matches(production).count(), 1);
    assert_eq!(WORKFLOW.matches(synthetic_allow).count(), 1);
    assert_eq!(WORKFLOW.matches(synthetic_deny).count(), 1);
    assert_eq!(WORKFLOW.matches("PRIVATE_CI_EXPORT_DENIED:").count(), 1);
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("permissions:\n  contents: read"));
    assert!(WORKFLOW.contains("bash verification/source-integrity/verify.sh"));
    assert!(!WORKFLOW.contains("Direct connector create_blob"));
}
