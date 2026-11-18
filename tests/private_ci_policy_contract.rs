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

fn only_comments_or_blank(source: &str) -> bool {
    source
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
}

#[test]
fn rust_policy_classifier_rejects_unapproved_destinations() {
    for accepted in [
        "",
        "\n",
        "# no destinations\n",
        "  \t \n # explanatory comment\n\t# second comment\n",
    ] {
        assert!(only_comments_or_blank(accepted), "valid disabled policy rejected");
    }
    for rejected in [
        "other-org/private-repo 123\n",
        "  other-org/private-repo 123\n",
        "# harmless\nother-org/private-repo 123\n",
        " # comment\n\tother-org/private-repo 123",
        "# comment\nrepo/evil 1 # disguised comment\n",
        "\0",
    ] {
        assert!(
            !only_comments_or_blank(rejected),
            "active destination escaped rejection"
        );
    }
}

#[test]
fn checked_in_allowlist_matches_rust_deny_default_contract() {
    assert!(ALLOWLIST.len() <= 65_536, "allowlist exceeds bounded policy size");
    assert!(only_comments_or_blank(ALLOWLIST));
    assert!(!ALLOWLIST.contains('\0'));
    assert!(WORKFLOW.contains("test ! -L \"$file\""));
    assert!(WORKFLOW.contains("test \"$(stat -c %s -- \"$file\")\" -le 65536"));
}
