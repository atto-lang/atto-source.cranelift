//! Real Cranelift JIT conformance against the same 22 source expressions
//! pinned in Java, LLVM, WASM, and .NET. This is not JVM/LLVM/NativeAOT proof.
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const MANIFEST: &str = include_str!("../contracts/parity/portable-scalar-v1-20261101.json");
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-portable-v1-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("cannot create isolated scalar test directory");
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run_source(dir: &Fixture, id: &str, expr: &str) -> Output {
    let source = dir.0.join(format!("{id}.ores"));
    fs::write(&source, format!("fnc main():int{{return {expr};}}\n"))
        .expect("cannot write scalar case");
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(source)
        .output()
        .expect("real compiled Cranelift CLI must execute")
}
fn assert_manifest_case(id: &str, expr: &str, expected: Option<&str>) {
    assert!(
        MANIFEST.contains(&format!("\"id\": \"{id}\"")),
        "manifest case missing: {id}"
    );
    assert!(
        MANIFEST.contains(&format!("\"expression\": \"{expr}\"")),
        "expression drift for {id}"
    );
    if let Some(value) = expected {
        assert!(
            MANIFEST.contains(&format!("\"expected\": \"{value}\"")),
            "expected i64 value absent for {id}"
        );
    }
}

#[test]
fn shared_manifest_inventory_cannot_silently_drop_cases() {
    assert!(MANIFEST.contains("\"schema_version\": \"2026-11-01.scalar-v1\""));
    assert!(MANIFEST.contains("\"scope\": \"import-free-pure-i64\""));
    assert_eq!(MANIFEST.matches("\"id\":").count(), 22);
    assert_eq!(MANIFEST.matches("\"expression\":").count(), 22);
    assert_eq!(MANIFEST.matches("\"expected\":").count(), 16);
    assert_eq!(MANIFEST.matches("\"reason\":").count(), 6);
}

#[test]
fn portable_shared_scalar_matrix_executes_in_real_cranelift_jit() {
    let fixture = Fixture::new();
    for (id, expr, expected) in [
        ("zero", "0", "0"),
        ("positive-forty-two", "6 * 7", "42"),
        ("negative-forty-two", "-7 * 6", "-42"),
        ("max-i64", "9223372036854775807", "9223372036854775807"),
        ("min-i64", "-9223372036854775808", "-9223372036854775808"),
        (
            "max-minus-one",
            "9223372036854775807 - 1",
            "9223372036854775806",
        ),
        (
            "min-plus-one",
            "-9223372036854775808 + 1",
            "-9223372036854775807",
        ),
        (
            "over-binary64-mantissa",
            "9007199254740993 - 9007199254740992",
            "1",
        ),
        (
            "negative-over-binary64",
            "-9007199254740993 - (-9007199254740992)",
            "-1",
        ),
        ("precedence", "2 + 3 * 4", "14"),
        ("parentheses", "(2 + 3) * 4", "20"),
        ("subtraction-left-assoc", "20 - 5 - 3", "12"),
        ("grouped-subtraction", "20 - (5 - 3)", "18"),
        ("double-negative", "-(-42)", "42"),
        ("two-negative-factors", "(-7) * (-6)", "42"),
        ("underscore-literal", "1_000 + 24", "1024"),
    ] {
        assert_manifest_case(id, expr, Some(expected));
        let result = run_source(&fixture, id, expr);
        assert!(
            result.status.success(),
            "{id} must execute; stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!("{expected}\n"),
            "wrong signed i64 JIT result: {id}"
        );
        assert!(
            result.stderr.is_empty(),
            "successful JIT case {id} unexpectedly logged a diagnostic"
        );
    }
}

#[test]
fn portable_shared_scalar_negatives_fail_closed_without_guest_stdout() {
    let fixture = Fixture::new();
    for (id, expr) in [
        ("div-int", "7 / 2"),
        // The historical v1 corpus forbade ^, but bitwise-v1 now defines
        // integer XOR; old negative is deliberately superseded by scalar-v4.
        ("forbidden-double-caret", "7 ^^ 3"),
        ("forbidden-remainder", "7 % 3"),
        ("i64-overflow-literal", "9223372036854775808"),
        ("negative-i64-underflow-literal", "-9223372036854775809"),
    ] {
        assert_manifest_case(id, expr, None);
        let result = run_source(&fixture, id, expr);
        assert!(!result.status.success(), "{id} must never be accepted");
        assert!(
            result.stdout.is_empty(),
            "{id} emitted stdout after rejection"
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("orescranelift:"),
            "{id} did not produce a compiler-owned diagnostic"
        );
    }
}
