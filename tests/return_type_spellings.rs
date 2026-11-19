//! All six scalar-i64 return spellings must lower to the same native result.
//! The current Cranelift profile does not support void/nominal return types.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Source(PathBuf);
impl Source {
    fn new(source: &str) -> Self {
        for _ in 0..64 {
            let i = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("atto-return-i64-{}-{i}.atto", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => { f.write_all(source.as_bytes()).unwrap(); return Self(path); }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot write return source: {e}"),
            }
        }
        panic!("source namespace exhausted")
    }
}
impl Drop for Source { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
fn invoke(mode: &str, source: &Source) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode).arg(&source.0).output().expect("run actual Cranelift CLI")
}

#[test]
fn all_six_scalar_i64_spellings_execute_equivalent_native_code() {
    let signatures = [
        ": int", "-> int", ": int ->",
        ": infer", "-> infer", ": infer ->",
    ];
    for spelling in signatures {
        let source = Source::new(&format!("fnc main() {spelling} {{ return 42; }}"));
        let checked = invoke("check", &source);
        assert_eq!(checked.status.code(), Some(0), "check {spelling}: {}", String::from_utf8_lossy(&checked.stderr));
        assert!(checked.stdout.is_empty());
        let clif = invoke("ir", &source);
        assert_eq!(clif.status.code(), Some(0), "ir {spelling}: {}", String::from_utf8_lossy(&clif.stderr));
        assert!(String::from_utf8_lossy(&clif.stdout).contains("function"));
        let native = invoke("run", &source);
        assert_eq!(native.status.code(), Some(0), "run {spelling}: {}", String::from_utf8_lossy(&native.stderr));
        assert_eq!(native.stdout, b"42\n", "incorrect native scalar result: {spelling}");
        assert!(native.stderr.is_empty());
    }
}

#[test]
fn unsupported_void_nominal_and_malformed_return_spelling_still_rejects() {
    for spelling in [
        ": void", "-> void", ": void ->",
        ": String", "-> Borrow<int>", "-> int ->",
        ": infer -", ": int -> ->", ": int infer",
    ] {
        let src = Source::new(&format!("fnc main() {spelling} {{ return 42; }}"));
        for mode in ["check", "ir", "run"] {
            let got = invoke(mode, &src);
            assert_eq!(got.status.code(), Some(1), "{mode} admitted {spelling}");
            assert!(got.stdout.is_empty(), "{mode} emitted value or CLIF for {spelling}");
            assert!(!got.stderr.is_empty(), "{mode} denied without compiler diagnostic: {spelling}");
        }
    }
}
