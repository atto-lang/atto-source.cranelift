//! Stage-0 Cranelift CLI/JIT checks for canonical scalar v5 (13 positive, 8 negative).
//! Exact-head cargo results are required before counting any native guest evidence.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const MANIFEST: &str = include_str!("../contracts/parity/portable-scalar-v5-20261120.json");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("atto-cranelift-v5-{}-{nonce}", std::process::id()));
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
fn scalar_v5_manifest_inventory_is_exact() {
    assert!(MANIFEST.contains("\"schema_version\": \"2026-11-20.scalar-v5\""));
    assert_eq!(MANIFEST.matches("\"id\":").count(), 21);
    assert_eq!(MANIFEST.matches("\"expression\":").count(), 21);
    assert_eq!(MANIFEST.matches("\"expected\":").count(), 13);
}

#[test]
fn scalar_v5_real_jit_positive_values() {
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
        ("bitxor-11-4", "11 ^^ 4", "15"),
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
fn is_native_checked_overflow(stderr: &[u8]) -> bool {
    stderr == b"orescranelift: i64 arithmetic overflow\n"
}

fn is_compiler_rejection(id: &str, stderr: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(stderr) else {
        return false;
    };
    // Percent is removed; obsolete single ^ infix is rejected before JIT,
    // not confused with a removed arithmetic operator.
    let prefix = match id {
        "removed-percent-new" => "orescranelift: invalid character at byte ",
        "obsolete-single-caret-infix" => "orescranelift: E_OPERATOR_CARET: single infix ^ is not XOR at byte ",
        _ => return text == "orescranelift: i64 literal out of range\n",
    };
    let Some(offset) = text.strip_prefix(prefix).and_then(|s| s.strip_suffix('\n')) else {
        return false;
    };
    !offset.is_empty() && offset.bytes().all(|byte| byte.is_ascii_digit())
}

#[test]
fn scalar_v5_rejection_phase_mutations_fail_closed() {
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
        "removed-percent-new",
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
fn scalar_v5_compiler_rejects_invalid_sources() {
    let fixture = Fixture::new();
    for (id, expression) in [
        ("removed-percent-new", "11 % 4"),
        ("obsolete-single-caret-infix", "11 ^ 4"),
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
fn scalar_v5_checked_overflow_fails_closed() {
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

#[test]
fn scalar_v5_statement_caret_generates_the_same_native_result_as_return() {
    let fixture = Fixture::new();
    let path = fixture.0.join("caret-return.atto");
    fs::write(&path, "pub fnc main():int{^ 11 ^^ 4;}\n").unwrap();
    for mode in ["run", "ir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&path)
            .output()
            .expect("source-compiled Cranelift CLI must execute");
        assert_eq!(output.status.code(), Some(0), "{mode}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stderr.is_empty(), "{mode} produced compiler diagnostics");
        if mode == "run" {
            assert_eq!(String::from_utf8_lossy(&output.stdout), "15\n");
        } else {
            assert!(String::from_utf8_lossy(&output.stdout).contains("function"),
                "source-to-CLIF emission missing");
        }
    }
}

#[test]
fn scalar_v5_infix_caret_and_unsupported_float_source_fail_before_native_ir() {
    let fixture = Fixture::new();
    for (index, statement) in [
        "return 11 ^ 4;",
        "return 11 ^ ^ 4;",
        "return 11 ^/*comment*/^ 4;",
        "return 5.0 / 2.0;",
    ].into_iter().enumerate() {
        let path = fixture.0.join(format!("reject-{index}.atto"));
        fs::write(&path, format!("pub fnc main():int{{{statement}}}\n")).unwrap();
        for mode in ["run", "ir"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .expect("source-compiled Cranelift CLI must report failure");
            assert_eq!(output.status.code(), Some(1), "{mode}/{statement}: {}", String::from_utf8_lossy(&output.stderr));
            assert!(output.stdout.is_empty(), "{mode}/{statement} emitted forbidden guest/IR");
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(diagnostic.starts_with("orescranelift: "), "{mode}/{statement}: {diagnostic}");
            if statement.contains('^') {
                assert!(diagnostic.contains("E_OPERATOR_CARET"), "{mode}/{statement}: {diagnostic}");
            }
        }
    }
}


// The November 23 decision supersedes the historical v5 float-quotient
// sentence. Native failure statuses do not prove guest Error/trap support.
#[test]
fn reconciled_signed_division_native_values_and_distinct_errors() {
    let fixture = Fixture::new();
    let path = fixture.0.join("signed-division.atto");
    fs::write(&path, "pub fnc main(a:int,b:int):int{return a / b;}\n").unwrap();
    for (a, b, expected) in [
        ("5", "2", "2"),
        ("-5", "2", "-2"),
        ("5", "-2", "-2"),
        ("-5", "-2", "2"),
        ("0", "7", "0"),
        ("-9223372036854775808", "1", "-9223372036854775808"),
        ("9223372036854775807", "1", "9223372036854775807"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .args([a, b])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{a}/{b}: {:?}", output.stderr);
        assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{expected}\n"));
        assert!(output.stderr.is_empty());
    }
    for (a, b, expected) in [
        ("1", "0", "integer divisor zero"),
        ("-9223372036854775808", "-1", "i64 arithmetic overflow"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .args([a, b])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{a}/{b}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!("orescranelift: {expected}\n")
        );
    }
}


// Keep this separate from the immutable historical v5 manifest. Runtime
// parameters prevent the literal-only corpus from hiding broken JIT lowering.
#[test]
fn canonical_xor_runtime_parameters_match_named_intrinsic() {
    let fixture = Fixture::new();
    let punctuation = fixture.0.join("runtime-xor.atto");
    let named = fixture.0.join("runtime-named-xor.atto");
    fs::write(&punctuation, "pub fnc main(a:int,b:int):int{^ a ^^ b;}\n").unwrap();
    fs::write(&named, "pub fnc main(a:int,b:int):int{return Integer.bitXor(a,b);}\n").unwrap();
    for (a, b, expected) in [
        (12i64, 10i64, 6i64),
        (i64::MIN, -1, i64::MAX),
        (i64::MAX, -1, i64::MIN),
        (i64::MIN, i64::MIN, 0),
        (i64::MIN, i64::MAX, -1),
        (i64::MAX, i64::MIN, -1),
        (0, i64::MIN, i64::MIN),
        (-1, 0, -1),
        (4_294_967_296, 1, 4_294_967_297),
        (9_007_199_254_740_993, 9_007_199_254_740_992, 1),
    ] {
        for path in [&punctuation, &named] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg("run")
                .arg(path)
                .args([a.to_string(), b.to_string()])
                .output()
                .expect("real Cranelift CLI must execute");
            assert_eq!(output.status.code(), Some(0), "{path:?} {a} ^^ {b}: {:?}", output.stderr);
            assert!(output.stderr.is_empty(), "{path:?} unexpected diagnostic");
            assert_eq!(output.stdout, format!("{expected}\n").as_bytes(),
                "{path:?} {a} ^^ {b}: wrong native value");
        }
    }
}

// Stage-0 does not yet have writable lexical slots or Bool. Explicitly
// preserve rejection before IR instead of claiming compound parity.
#[test]
fn unsupported_compound_xor_and_boolean_caret_never_emit_native_ir() {
    let fixture = Fixture::new();
    for (index, body) in [
        "let int x = 12; x ^^= 10; return x;",
        "val int x = 12; x ^^= 10; return x;",
        "return true ^^ false;",
        "return 12 ^^ true;",
        "return 12 ^^= 10;",
        "return 12 ^^ = 10;",
    ].into_iter().enumerate() {
        let path = fixture.0.join(format!("unsupported-xor-{index}.atto"));
        fs::write(&path, format!("pub fnc main():int{{{body}}}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("ir")
            .arg(&path)
            .output()
            .expect("real Cranelift CLI must report source rejection");
        assert_eq!(output.status.code(), Some(1), "{body}: {:?}", output.stderr);
        assert!(output.stdout.is_empty(), "{body} emitted forbidden native IR");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "),
            "{body} missing compiler diagnostic");
    }
}
