//! Stage-0 Cranelift native-JIT slice of the reviewed portable scalar v2 corpus.
//! This is authored evidence until exact-head cargo tests run with nonzero CI steps.
//! It does NOT claim the full Oreslang stdlib/TestSuite or actor ABI.
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const ORACLE: &str = include_str!("../contracts/parity/portable-scalar-v2-20261101.json");

fn native_case(id: &str, expression: &str, expected: Option<&str>) {
    // Source/expected values are independently pinned in the shared corpus.
    // A substring guard is not a JSON parser and is not execution evidence.
    assert!(
        ORACLE.contains(&format!("\"id\": \"{id}\"")),
        "unknown corpus ID"
    );
    assert!(
        ORACLE.contains(&format!("\"expression\": \"{expression}\"")),
        "expression drift from shared corpus: {id}"
    );
    if let Some(expected) = expected {
        assert!(
            ORACLE.contains(&format!("\"expected\": \"{expected}\"")),
            "expected value drift from shared corpus: {id}"
        );
    }
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "orescranelift-parity-v2-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).expect("private fixture dir must be new");
    let source_path = dir.join("case.ores");
    let mut source_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&source_path)
        .expect("fixture source must not already exist");
    writeln!(source_file, "pub fnc main():int{{return {expression};}}").unwrap();
    source_file.flush().unwrap();
    drop(source_file);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&source_path)
        .output()
        .expect("real Cranelift CLI failed to spawn");
    std::fs::remove_dir_all(dir).unwrap();
    match expected {
        Some(value) => {
            assert!(
                output.status.success(),
                "{id}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("{value}\n"),
                "{id}"
            );
            assert!(
                output.stderr.is_empty(),
                "{id}: unexpected guest diagnostic"
            );
        }
        None => {
            assert!(
                !output.status.success(),
                "{id}: invalid guest source accepted"
            );
            assert!(
                output.stdout.is_empty(),
                "{id}: rejected source emitted guest stdout"
            );
            assert!(
                !output.stderr.is_empty(),
                "{id}: rejection has no compiler diagnostic"
            );
        }
    }
}

#[test]
fn portable_scalar_v2_native_signed_i64_values() {
    for (id, expression, expected) in [
        (
            "max-plus-negative-one",
            "9223372036854775807 + (-1)",
            "9223372036854775806",
        ),
        (
            "min-minus-negative-one",
            "-9223372036854775808 - (-1)",
            "-9223372036854775807",
        ),
        (
            "large-roundtrip",
            "(9007199254740993 + 7) - 9007199254740993",
            "7",
        ),
        (
            "large-negative-roundtrip",
            "(-9007199254740993 - 7) - (-9007199254740993)",
            "-7",
        ),
        ("right-nested-subtraction", "100 - (50 - (20 - 7))", "63"),
        ("left-nested-subtraction", "((100 - 50) - 20) - 7", "23"),
        (
            "mul-near-max",
            "3037000499 * 3037000499",
            "9223372030926249001",
        ),
        (
            "mul-near-min",
            "(-3037000499) * 3037000499",
            "-9223372030926249001",
        ),
        ("triple-negative", "-(-(-42))", "-42"),
        (
            "signed-underscore",
            "-9_223_372_036_854_775_807 + 8",
            "-9223372036854775799",
        ),
    ] {
        native_case(id, expression, Some(expected));
    }
}

#[test]
fn portable_scalar_v2_compiler_negative_sources() {
    assert!(ORACLE.contains("\\\"expression\\\": \\\"7 ^^ 3\\\""));
    for (id, expression) in [
        ("removed-percent", "7 % 3"),
        // Legacy v2's ^^ negative is superseded by canonical integer XOR.
        // It remains in the immutable v2 manifest but is no longer executed
        // as a current compiler rejection. See scalar-v5 positive guests.
        ("out-of-range-positive", "9223372036854775808"),
    ] {
        native_case(id, expression, None);
    }
}
