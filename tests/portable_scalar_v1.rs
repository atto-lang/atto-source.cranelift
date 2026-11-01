//! Actual Cranelift native JIT execution of the shared 16+6 pure-i64 contract.
//! Shared contract is byte-identical to Java, LLVM, Wasm, and .NET candidates.
//! No host arithmetic oracle, no stdlib/actor claim, and no claim of hosted CI.
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn native_i64_v1_contract_with_negative_admission() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contracts")
        .join("parity")
        .join("portable-scalar-v1-20261101.json");
    let metadata = std::fs::symlink_metadata(&manifest).expect("missing shared scalar contract");
    assert!(metadata.file_type().is_file(), "contract must be a regular file");
    let hash = Command::new("git")
        .arg("hash-object")
        .arg("--")
        .arg(&manifest)
        .output()
        .expect("git required for exact-byte conformance pin");
    assert!(hash.status.success(), "failed to fingerprint scalar contract");
    assert_eq!(
        String::from_utf8_lossy(&hash.stdout).trim(),
        "001e8894cbeb87509878a4de36263a1a327805b1",
        "the five-backend scalar contract has drifted"
    );

    // These expressions are transcribed exactly from the pinned immutable
    // contract. A changed contract blob must fail before invoking any guest.
    let positive = [
        ("zero", "0", "0"),
        ("positive-forty-two", "6 * 7", "42"),
        ("negative-forty-two", "-7 * 6", "-42"),
        ("max-i64", "9223372036854775807", "9223372036854775807"),
        ("min-i64", "-9223372036854775808", "-9223372036854775808"),
        ("max-minus-one", "9223372036854775807 - 1", "9223372036854775806"),
        ("min-plus-one", "-9223372036854775808 + 1", "-9223372036854775807"),
        ("over-binary64-mantissa", "9007199254740993 - 9007199254740992", "1"),
        ("negative-over-binary64", "-9007199254740993 - (-9007199254740992)", "-1"),
        ("precedence", "2 + 3 * 4", "14"),
        ("parentheses", "(2 + 3) * 4", "20"),
        ("subtraction-left-assoc", "20 - 5 - 3", "12"),
        ("grouped-subtraction", "20 - (5 - 3)", "18"),
        ("double-negative", "-(-42)", "42"),
        ("two-negative-factors", "(-7) * (-6)", "42"),
        ("underscore-literal", "1_000 + 24", "1024"),
    ];
    let negative = [
        ("div-int", "7 / 2", "invalid character"),
        ("forbidden-caret", "7 ^ 3", "invalid character"),
        ("forbidden-double-caret", "7 ^^ 3", "invalid character"),
        ("forbidden-remainder", "7 % 3", "operator '%' is removed"),
        ("i64-overflow-literal", "9223372036854775808", "i64 literal out of range"),
        ("negative-i64-underflow-literal", "-9223372036854775809", "i64 literal out of range"),
    ];
    assert_eq!(positive.len(), 16);
    assert_eq!(negative.len(), 6);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_nanos();
    let workspace = std::env::temp_dir().join(format!(
        "orescranelift-portable-scalar-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir(&workspace).expect("cannot create isolated fixture directory");

    for (i, (id, expression, expected)) in positive.iter().enumerate() {
        let path = workspace.join(format!("positive-{i:02}-{id}.ores"));
        std::fs::write(
            &path,
            format!("pub fnc main(): int {{ return {expression}; }}\n"),
        )
        .expect("cannot write guest fixture");
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .output()
            .expect("native CLI did not start");
        assert!(
            output.status.success(),
            "native JIT rejected positive case {id}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.stdout,
            format!("{expected}\n").into_bytes(),
            "incorrect native i64 value for {id}"
        );
    }
    for (i, (id, expression, diagnostic)) in negative.iter().enumerate() {
        let path = workspace.join(format!("negative-{i:02}-{id}.ores"));
        std::fs::write(
            &path,
            format!("pub fnc main(): int {{ return {expression}; }}\n"),
        )
        .expect("cannot write negative fixture");
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .output()
            .expect("native CLI did not start");
        assert!(!output.status.success(), "native JIT admitted invalid case {id}");
        assert!(output.stdout.is_empty(), "invalid case {id} printed stdout");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "wrong rejection for {id}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(&workspace).expect("cannot remove test fixture directory");
}
