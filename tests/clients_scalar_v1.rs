//! Exact-source client scalar profile: native Cranelift JIT execution and
//! pre-lowering negative denial. These are NOT full HTTP or SDK tests.
use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str, expected: Option<&str>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/fixtures/clients-scalar-v1").join(name);
    assert!(source.is_file(), "missing fixture: {}", source.display());
    assert!(!source.symlink_metadata().unwrap().file_type().is_symlink());
    let contents = std::fs::read(&source).unwrap();
    assert!(contents.len() > 20 && contents.len() < 2048);
    assert!(!contents.contains(&0));
    let src = std::str::from_utf8(&contents).unwrap();
    assert!(
        src.contains("fnc main(): int"),
        "not the canonical scalar source"
    );

    let binary = env!("CARGO_BIN_EXE_orescranelift");
    // The native frontend/IR check must reject negatives without emitting IR.
    let ir = Command::new(binary)
        .arg("ir")
        .arg(&source)
        .output()
        .unwrap();
    let run = Command::new(binary)
        .arg("run")
        .arg(&source)
        .output()
        .unwrap();

    match expected {
        Some(value) => {
            assert!(
                ir.status.success(),
                "IR rejected {}: {}",
                name,
                String::from_utf8_lossy(&ir.stderr)
            );
            assert!(
                !ir.stdout.is_empty(),
                "IR emitted no native code for {name}"
            );
            assert!(
                run.status.success(),
                "JIT rejected {}: {}",
                name,
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(
                run.stdout,
                format!("{value}\n").as_bytes(),
                "wrong guest output: {name}"
            );
            assert!(
                run.stderr.is_empty(),
                "guest emitted unexpected stderr: {name}"
            );
        }
        None => {
            for (phase, result) in [("IR", &ir), ("JIT", &run)] {
                assert!(
                    !result.status.success(),
                    "{phase} accepted rejected guest: {name}"
                );
                assert!(
                    result.stdout.is_empty(),
                    "{phase} emitted output for rejected guest: {name}"
                );
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(
                    !diagnostic.trim().is_empty(),
                    "{phase} denial missing diagnostic: {name}"
                );
                assert_eq!(
                    result.status.code(),
                    Some(1),
                    "{phase} compiler exited abnormally instead of rejecting {name}"
                );
                assert!(
                    diagnostic.contains("orescranelift: "),
                    "{phase} rejected source without compiler-owned diagnostic: {name}"
                );
                let required = match name {
                    "07-unknown-reject.ores" => "unbound or unsupported identifier",
                    "08-overflow-reject.ores" => "i64 literal out of range",
                    "09-syntax-reject.ores" => "expected scalar expression",
                    _ => panic!("unexpected negative fixture: {name}"),
                };
                assert!(
                    diagnostic.contains(required),
                    "{phase} wrong rejection category for {name}: {diagnostic}"
                );
                assert!(
                    !diagnostic.contains("panicked at") && !diagnostic.contains("stack backtrace"),
                    "{phase} compiler crashed instead of rejecting {name}: {diagnostic}"
                );
            }
        }
    }
}

#[test]
fn client_scalar_precedence() {
    fixture("01-precedence.ores", Some("14"));
}
#[test]
fn client_scalar_grouping() {
    fixture("02-grouping.ores", Some("20"));
}
#[test]
fn client_scalar_signed() {
    fixture("03-signed.ores", Some("5"));
}
#[test]
fn client_scalar_i64_edge() {
    fixture("04-i64-edge.ores", Some("1"));
}
#[test]
fn client_scalar_digit_separators() {
    fixture("05-separators.ores", Some("100"));
}
#[test]
fn client_scalar_nested() {
    fixture("06-nested.ores", Some("35"));
}
#[test]
fn client_scalar_unknown_symbol_rejected() {
    fixture("07-unknown-reject.ores", None);
}
#[test]
fn client_scalar_out_of_range_rejected() {
    fixture("08-overflow-reject.ores", None);
}
#[test]
fn client_scalar_invalid_syntax_rejected() {
    fixture("09-syntax-reject.ores", None);
}
