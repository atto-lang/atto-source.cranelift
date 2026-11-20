//! Actual scalar Cranelift binary admission and execution for restricted inline tasks.
//! Not evidence of scheduling, TaskId, cancellation, ownership or TenantVM support.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Source(PathBuf);
impl Source {
    fn new(text: &str) -> Self {
        for _ in 0..64 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-inline-task-{}-{id}.atto", std::process::id()
            ));
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(text.as_bytes()).expect("write fixture");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture setup: {e}"),
            }
        }
        panic!("fixture namespace exhausted");
    }
}
impl Drop for Source {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}
fn call(mode: &str, fixture: &Source, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode).arg(&fixture.0).args(args)
        .output().expect("invoke real Cranelift CLI")
}

#[test]
fn inline_scalar_guest_is_sequential_and_emits_real_clif() {
    for (source, args, expected) in [
        ("fnc main():int { sync task {}; return 42; }", &[][..], "42\n"),
        ("fnc main():int { do sync task<int> { return 8; }; return sync task<int> { return 6*7; }; }", &[], "42\n"),
        ("fnc main(v:int):int { return sync task<int> { return v + 2; }; }", &["40"][..], "42\n"),
        ("fnc main():int { return sync task { return sync task<int> { return 42; }; }; }", &[], "42\n"),
    ] {
        let fixture = Source::new(source);
        let checked = call("check", &fixture, &[]);
        assert!(checked.status.success(), "check: {}", String::from_utf8_lossy(&checked.stderr));
        assert!(checked.stdout.is_empty());
        let ir = call("ir", &fixture, &[]);
        assert!(ir.status.success(), "CLIF: {}", String::from_utf8_lossy(&ir.stderr));
        assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));
        let native = call("run", &fixture, args);
        assert!(native.status.success(), "guest: {}", String::from_utf8_lossy(&native.stderr));
        assert_eq!(native.stdout, expected.as_bytes(), "guest {source}");
    }
}

#[test]
fn discarded_inline_expression_must_execute_and_preserve_failure_precedence() {
    let fixture = Source::new(
        "fnc main():int { do sync task<int> { return Integer.rem(5,0); }; return 42; }"
    );
    let checked = call("check", &fixture, &[]);
    assert!(checked.status.success(), "parse must admit the checked scalar");
    let native = call("run", &fixture, &[]);
    assert_eq!(native.status.code(), Some(1));
    assert!(native.stdout.is_empty(), "discarded task must not publish 42");
    assert!(String::from_utf8_lossy(&native.stderr).contains("integer divisor zero"));
}

#[test]
fn unverified_spawn_suspend_and_payload_forms_produce_no_guest_artifacts() {
    for source in [
        "fnc main():int { return task<int> { return 42; }; }",
        "fnc main():int { do task<int> { return 42; } return 0; }",
        "fnc main():int { return await task<int> { return 42; }; }",
        "fnc main():int { return sync task {}; }",
        "fnc main():int { return sync task<void> { return; }; }",
        "fnc main():int { return sync task<int> { return; }; }",
        "fnc main():int { return sync task<int> { defer x; return 42; }; }",
        "fnc main():int { return sync task<int> { return task<int> {return 42;}; }; }",
        "fnc main(sync:int):int { return sync; }",
        "fnc main():int { return synchronize task<int> { return 42; }; }",
    ] {
        let fixture = Source::new(source);
        for mode in ["check", "ir", "run"] {
            let got = call(mode, &fixture, &[]);
            assert_eq!(got.status.code(), Some(1), "{mode} admitted {source}");
            assert!(got.stdout.is_empty(), "{mode} emitted guest artifact for {source}");
            assert!(!got.stderr.is_empty(), "{mode} rejected without diagnostic");
        }
    }
}
