//! Stage-0 Cranelift CLI/JIT checks for scalar v3 (12 positive, 9 negative).
//! Exact-head cargo results are required before counting any native guest evidence.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const MANIFEST: &str = include_str!("../contracts/parity/portable-scalar-v3-20261101.json");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("atto-cranelift-v3-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).expect("cannot create isolated fixture directory");
        Self(directory)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_one(fixture: &Fixture, id: &str, expression: &str) -> std::process::Output {
    // Bind each expression to its own ID; two independent substring probes
    // cannot detect swapped expressions between names.
    let binding = format!("\"id\": \"{id}\",\n      \"expression\": \"{expression}\"");
    assert!(
        MANIFEST.contains(&binding),
        "manifest ID/expression binding drift for {id}"
    );
    let path = fixture.0.join(format!("{id}.atto"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("fixture source must be newly created");
    writeln!(file, "pub fnc main():int{{return {expression};}}").expect("write guest source");
    file.flush().expect("flush source");
    drop(file);
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("real Cranelift compiler/JIT must spawn")
}

#[test]
fn scalar_v3_manifest_inventory_is_exact() {
    assert!(MANIFEST.contains("\"schema_version\": \"2026-11-01.scalar-v3\""));
    assert_eq!(MANIFEST.matches("\"id\":").count(), 21);
    assert_eq!(MANIFEST.matches("\"expression\":").count(), 21);
    assert_eq!(MANIFEST.matches("\"expected\":").count(), 12);
}

#[test]
fn scalar_v3_real_jit_positive_values() {
    let fixture = Fixture::new();
    for (id, expression, expected) in [
        (
            "near-max-sum",
            "9223372036854775805 + 2",
            "9223372036854775807",
        ),
        (
            "near-min-successor-two",
            "(-9223372036854775807 - 1) + 2",
            "-9223372036854775806",
        ),
        (
            "wide-precision-multiply",
            "(9007199254740993 - 9007199254740992) * 41",
            "41",
        ),
        (
            "signed-precision-three",
            "-9007199254740995 + 9007199254740992",
            "-3",
        ),
        (
            "trillion-zero-cancel",
            "1000000000000000 + (-1000000000000000)",
            "0",
        ),
        ("mixed-precedence", "5 + 7 * 9 - 4 * 3", "56"),
        ("right-nested-subtraction-v3", "80 - (50 - (25 - 5))", "50"),
        ("left-nested-subtraction-v3", "((80 - 50) - 25) - 5", "0"),
        (
            "wide-positive-product",
            "2147483647 * 2147483647",
            "4611686014132420609",
        ),
        (
            "wide-negative-product",
            "(-2147483647) * 2147483647",
            "-4611686014132420609",
        ),
        ("quadruple-negation", "-(-(-(-33)))", "33"),
        (
            "max-range-roundtrip",
            "(9223372036854775800 - 9007199254740993) + 9007199254740993",
            "9223372036854775800",
        ),
    ] {
        let expected_binding =
            format!("\"expression\": \"{expression}\",\n      \"expected\": \"{expected}\"");
        assert!(
            MANIFEST.contains(&expected_binding),
            "manifest expression/expected value drift for {id}"
        );
        let output = run_one(&fixture, id, expression);
        assert!(
            output.status.success(),
            "{id} JIT failure: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{expected}\n"),
            "{id} wrong native guest value"
        );
        assert!(output.stderr.is_empty(), "{id} unexpected diagnostic");
    }
}

// Category-specific diagnostics are evidence of the language phase reached.
// A parser error, CLI usage failure or killed process is not a JIT overflow.
fn is_compiler_rejection(id: &str, stderr: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(stderr) else {
        return false;
    };
    if id.starts_with("removed-") {
        let Some(offset) = text
            .strip_prefix("orescranelift: invalid character at byte ")
            .and_then(|s| s.strip_suffix('\n'))
        else {
            return false;
        };
        !offset.is_empty() && offset.bytes().all(|c| c.is_ascii_digit())
    } else {
        text == "orescranelift: i64 literal out of range\n"
    }
}

fn is_native_checked_overflow(stderr: &[u8]) -> bool {
    stderr == b"orescranelift: i64 arithmetic overflow\n"
}

#[test]
fn scalar_v3_rejection_phase_mutations_fail_closed() {
    for bad in [
        "",
        "orescranelift: i64 arithmetic overflow\n",
        "orescranelift: missing source file\n",
        "orescranelift: invalid character at byte nope\n",
        "orescranelift: invalid character at byte 2\nextra",
        "signal: segmentation fault",
    ] {
        assert!(!is_compiler_rejection(
            "removed-percent-new",
            bad.as_bytes()
        ));
        assert!(!is_compiler_rejection(
            "positive-outside-signed-range",
            bad.as_bytes()
        ));
    }
    assert!(is_compiler_rejection(
        "removed-caret-new",
        b"orescranelift: invalid character at byte 27\n"
    ));
    assert!(is_compiler_rejection(
        "negative-outside-signed-range",
        b"orescranelift: i64 literal out of range\n"
    ));
    for bad in [
        b"orescranelift: i64 literal out of range\n".as_slice(),
        b"orescranelift: missing source file\n",
        b"orescranelift: i64 arithmetic overflow\nextra",
        b"orescranelift: i64 arithmetic overflow",
    ] {
        assert!(!is_native_checked_overflow(bad));
    }
    assert!(is_native_checked_overflow(
        b"orescranelift: i64 arithmetic overflow\n"
    ));
}

#[test]
fn scalar_v3_compiler_rejects_invalid_sources() {
    let fixture = Fixture::new();
    for (id, expression) in [
        ("removed-percent-new", "11 % 4"),
        ("removed-caret-new", "11 ^ 4"),
        ("removed-double-caret-new", "11 ^^ 4"),
        ("positive-outside-signed-range", "9223372036854775808 + 1"),
        ("negative-outside-signed-range", "-9223372036854775809"),
    ] {
        let output = run_one(&fixture, id, expression);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{id} must be compiler rejection exit 1"
        );
        assert!(output.stdout.is_empty(), "{id} emitted guest stdout");
        assert!(
            is_compiler_rejection(id, &output.stderr),
            "{id} was not a phase-specific compiler rejection: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn scalar_v3_checked_overflow_fails_closed() {
    // The native backend uses an explicit JIT overflow status and prints the
    // exact diagnostic. Do not count parser, linker or infrastructure failures.
    let fixture = Fixture::new();
    for (id, expression) in [
        ("max-add-two-overflow", "9223372036854775807 + 2"),
        ("min-minus-one-overflow", "(-9223372036854775807 - 1) - 1"),
        ("positive-square-overflow", "3037000500 * 3037000500"),
        ("negative-square-overflow", "(-3037000500) * 3037000500"),
    ] {
        let output = run_one(&fixture, id, expression);
        assert_eq!(output.status.code(), Some(1), "{id} overflow must exit 1");
        assert!(output.stdout.is_empty(), "{id} emitted a wrapped value");
        assert!(
            is_native_checked_overflow(&output.stderr),
            "{id} was not observed native JIT overflow: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
