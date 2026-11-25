//! Stage-0 native guest rejection of unsupported Atto ownership/lifetime syntax.
//! The 22-case shared MIR model is not executable borrow-checker parity.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const CONTRACT: &str = include_str!("../contracts/borrow-mir-v1/semantic-cases.json");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct GuestSource(PathBuf);
impl GuestSource {
    fn new(source: &str) -> Self {
        for _ in 0..64 {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-cranelift-ownership-mir-{}-{number}.atto",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(source.as_bytes())
                        .expect("write exclusive .atto fixture");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot reserve ownership fixture: {e}"),
            }
        }
        panic!("temporary fixture namespace exhausted");
    }
}
impl Drop for GuestSource {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn execute(mode: &str, source: &GuestSource) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&source.0)
        .output()
        .expect("run real compiled Cranelift CLI")
}

#[test]
fn shared_borrow_mir_contract_is_pinned_and_uncertified() {
    assert!(CONTRACT.contains("\"schema\": \"atto-borrow-conformance-v1\""));
    assert!(CONTRACT.contains("\"certification\": \"NO_EXECUTED_BACKEND_EVIDENCE\""));
    assert!(CONTRACT.contains("\"scope\": \"restricted-acyclic-typed-ownership-CFG-model\""));
    assert!(CONTRACT.contains("\"cranelift\""));
    assert!(CONTRACT.contains("\"status\": \"NOT_EXECUTED\""));
    for number in 1..=22 {
        let id = format!("\"id\": \"C{number:02}\"");
        assert_eq!(
            CONTRACT.matches(id.as_str()).count(),
            1,
            "missing/duplicate MIR case {id}"
        );
    }
}

#[test]
fn ownership_and_lifetime_syntax_fail_closed_until_real_mir_lowering() {
    // True native guest positive control: a reject-everything compiler is not useful.
    let control = GuestSource::new("fnc main():int{return 40+2;}\n");
    let run = execute("run", &control);
    assert_eq!(
        run.status.code(),
        Some(0),
        "positive scalar guest: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"42\n");
    assert!(run.stderr.is_empty());

    for (expression, specific_diagnostic) in [
        ("borrow! value", true),
        ("borrow! mut value", true),
        ("mut borrow! value", true),
        ("take! value", true),
        ("share! value", true),
        ("copy! value", true),
        ("copy!! value", true), // Deep graph copy is NOT scalar copy.
        ("lt! a", false),
        ("lifetime! a", false),
        ("Borrow<int,lt=a>(value)", false),
    ] {
        let source = GuestSource::new(&format!("fnc main():int{{return {expression};}}\n"));
        for mode in ["run", "ir"] {
            let output = execute(mode, &source);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{mode} accepted unsupported {expression}"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} published scalar value/CLIF for {expression}"
            );
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                diagnostic.starts_with("orescranelift: "),
                "compiler-owned diagnostic absent for {expression}: {diagnostic}"
            );
            assert!(
                !diagnostic.contains("panicked at"),
                "compiler crashed on {expression}"
            );
            if specific_diagnostic {
                assert!(
                    diagnostic.contains("OWNERSHIP_BANG_BACKEND_UNSUPPORTED"),
                    "wrong ownership gate: {expression}: {diagnostic}"
                );
            }
        }
    }

    let inert = GuestSource::new("fnc main():int{// borrow! mut, copy!! and lt! a\nreturn 42;}\n");
    let output = execute("run", &inert);
    assert_eq!(
        output.status.code(),
        Some(0),
        "comment is inert: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
}
