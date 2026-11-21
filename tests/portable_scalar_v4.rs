//! IMMUTABLE HISTORICAL scalar-v4 fixture identity (2026-11-19).
//! The old v4 language assigned infix ^ to integer XOR and rejected integer
//! ^^, both superseded by canonical atto-spec #143/#151. Keep the exact
//! manifest bytes and its test inventory, but NEVER run those obsolete guest
//! expectations against the canonical current compiler or label them parity.
//! Current compiled Cranelift guest coverage moved to portable_scalar_v5.rs.
const HISTORICAL: &str = include_str!("../contracts/parity/portable-scalar-v4-20261119.json");

#[test]
fn historical_v4_is_unchanged_and_explicitly_noncurrent() {
    assert!(HISTORICAL.contains("\"schema_version\": \"2026-11-19.scalar-v4\""));
    assert!(HISTORICAL.contains("\"status\": \"AUTHORED_ONLY_NOT_EXECUTED\""));
    assert!(HISTORICAL.contains("\"expression\": \"11 ^ 4\""));
    assert!(HISTORICAL.contains("\"expression\": \"11 ^^ 4\""));
    assert_eq!(HISTORICAL.matches("\"id\":").count(), 21);
    assert_eq!(HISTORICAL.matches("\"expression\":").count(), 21);
    assert_eq!(HISTORICAL.matches("\"expected\":").count(), 13);
    // Canonical v5 is independently checked via actual compiled JIT guests.
    let current = include_str!("../contracts/parity/portable-scalar-v5-20261120.json");
    assert!(current.contains("\"schema_version\": \"2026-11-20.scalar-v5\""));
    assert!(current.contains("\"expression\": \"11 ^^ 4\""));
    assert!(current.contains("\"expression\": \"11 ^ 4\""));
}
