use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn source(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contracts/platform-core-v1")
        .join(name)
}
fn invoke(name: &str) -> Output {
    let file = source(name);
    assert!(
        file.is_file(),
        "missing committed Oreslang fixture: {}",
        file.display()
    );
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(file)
        .output()
        .expect("execute the actual Oreslang Cranelift CLI")
}
fn succeeds(name: &str, expected_stdout: &str) {
    let result = invoke(name);
    assert!(
        result.status.success(),
        "positive source rejected: {name}; diagnostic: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        expected_stdout.as_bytes(),
        "wrong native JIT result for {name}"
    );
    assert!(
        result.stderr.is_empty(),
        "success emitted diagnostic for {name}"
    );
}
fn rejects(name: &str) {
    let result = invoke(name);
    assert!(!result.status.success(), "invalid source accepted: {name}");
    assert!(
        result.stdout.is_empty(),
        "invalid source printed success stdout: {name}"
    );
    assert!(
        !result.stderr.is_empty(),
        "invalid source had no diagnostic: {name}"
    );
    assert!(
        result.stderr.len() < 65536,
        "unbounded rejection diagnostic: {name}"
    );
}
#[test]
fn arith_precedence() {
    succeeds("arith-precedence.ores", "32\n");
}

#[test]
fn signed_minimum() {
    succeeds("signed-minimum.ores", "-9223372036854775808\n");
}

#[test]
fn max_minus_five() {
    succeeds("max-minus-five.ores", "9223372036854775802\n");
}

#[test]
fn two_negative_factors() {
    succeeds("two-negative-factors.ores", "42\n");
}

#[test]
fn nested_unary() {
    succeeds("nested-unary.ores", "42\n");
}

#[test]
fn negative_product() {
    succeeds("negative-product.ores", "-42\n");
}

#[test]
fn eager_checked_int() {
    succeeds("eager-checked-int.ores", "42\n");
}

#[test]
fn reject_unknown_symbol() {
    rejects("reject-unknown-symbol.ores");
}

#[test]
fn reject_string_as_int() {
    rejects("reject-string-as-int.ores");
}

#[test]
fn reject_overflow() {
    rejects("reject-overflow.ores");
}

#[test]
fn reject_xor_operator() {
    rejects("reject-xor-operator.ores");
}

#[test]
fn reject_no_value_return() {
    rejects("reject-no-value-return.ores");
}

#[test]
fn stdlib_core_suite_not_mislabeled_as_cranelift_stage0_support() {
    rejects("native-testing-core.ores");
}
