//! Checks validate only the stage-0 frontend, never guest compilation/parity.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Source(PathBuf);
impl Source {
    fn new(contents: &str) -> Self {
        for _ in 0..64 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-cranelift-check-{}-{id}.atto", std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(contents.as_bytes()).unwrap();
                    return Self(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot open check fixture: {error}"),
            }
        }
        panic!("exhausted check fixture namespace");
    }
}
impl Drop for Source {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

fn check(contents: &str) -> std::process::Output {
    let source = Source::new(contents);
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("check").arg(source.0.as_os_str()).output()
        .expect("launch actual compiler frontend")
}

#[test]
fn check_admits_stage_zero_scalar_without_ir_or_guest_output() {
    for src in [
        "fnc main():int{return 42;}",
        "fnc main(a:int):int{return Integer.bitXor(a,2);}",
        "fnc main():int { return 9223372036854775807 - 9223372036854775765; }",
    ] {
        let output = check(src);
        assert_eq!(output.status.code(), Some(0), "parser denied valid scalar: {}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stdout.is_empty(), "check must not emit native result or CLIF");
        assert!(output.stderr.is_empty(), "check must not emit diagnostics on success");
    }
}

#[test]
fn check_refuses_actor_ownership_and_malformed_source() {
    for src in [
        "fnc main():int{return borrow! value;}",
        "define actor Worker as receive(m:int) -> void {} end",
        "fnc main():int{return 5 ^ 3;}",
        "fnc main():int{return never_declared;}",
    ] {
        let output = check(src);
        assert_eq!(output.status.code(), Some(1), "check admitted unsupported source: {src}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "));
    }
}

#[test]
fn check_rejects_runtime_arguments_and_missing_sources_before_guest() {
    let source = Source::new("fnc main():int{return 42;}");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("check").arg(&source.0).arg("17").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not accept runtime arguments"));
    let missing = Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("check").output().unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
}
