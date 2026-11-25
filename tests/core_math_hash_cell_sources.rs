//! Cranelift-owned .atto source integrity/reference only; not JIT execution.
const MATH: &str = include_str!("../library/atto.core/math.atto");
const HASH: &str = include_str!("../library/atto.core/hash.atto");
const CELL: &str = include_str!("../library/atto.core/cell.atto");
fn fold(state: i64, octet: i64) -> i64 {
    if state < 0 || state >= 1_000_000_007 || octet < 0 || octet > 255 {
        return -1;
    }
    let mut v = state * 31 + octet;
    for _ in 0..31 {
        if v >= 1_000_000_007 {
            v -= 1_000_000_007;
        }
    }
    v
}
#[test]
fn pure_native_core_source_is_owned_not_ambient() {
    for (name, source) in [("math", MATH), ("hash", HASH), ("cell", CELL)] {
        assert!(source.contains(&format!("define module {name} as")));
        assert!(source.len() <= 1_048_576);
        assert!(!source.contains("java:"));
        assert!(!source.contains("dotnet:"));
        assert!(!source.contains("import module std"));
    }
}
#[test]
fn fold_reference_vectors_not_compiled_guest() {
    for state in [0, 1, 17, 569, 1_000_000_006] {
        for octet in 0..=255 {
            assert_eq!(fold(state, octet), (state * 31 + octet) % 1_000_000_007);
        }
    }
    assert_eq!(fold(fold(17, 42), 7), 17_646);
    assert_eq!(fold(17, 256), -1);
    assert_eq!(fold(-1, 1), -1);
}
#[test]
fn pure_policy_never_grants_cell_mutability() {
    assert!(CELL.contains("not real Cell<T>/RefCell<T>"));
    assert!(CELL.contains("do NOT grant loans"));
}

#[test]
fn no_core_policy_predicate_is_a_borrow_capability_or_guest_proof() {
    for method in [
        "valid_loan_state",
        "can_owner_read",
        "can_owner_write",
        "can_move_owner",
        "can_suspend_unpinned",
    ] {
        assert!(
            CELL.contains(&format!("pub fnc {method}(")),
            "missing pure owner policy in Cranelift-owned Atto source: {method}"
        );
    }
    for [readers, writer_active, valid, writable] in [
        [0, 0, 1, 1],
        [1, 0, 1, 0],
        [65535, 0, 1, 0],
        [65536, 0, 0, 0],
        [0, 1, 1, 0],
        [1, 1, 0, 0],
        [-1, 0, 0, 0],
    ] {
        let valid_state = readers >= 0
            && readers <= 65535
            && (!writer_active_is_true(writer_active) || readers == 0);
        let owner_can_move = readers == 0 && !writer_active_is_true(writer_active);
        assert_eq!(valid_state, valid == 1);
        assert_eq!(owner_can_move, writable == 1);
    }
    // Host-only independent model deliberately confers no source-level loan.
    assert!(CELL.contains("NON-CAPABILITIES"));
}
fn writer_active_is_true(value: i32) -> bool {
    value != 0
}
