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
            let path = std::env::temp_dir()
                .join(format!("atto-inline-task-{}-{id}.atto", std::process::id()));
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
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn call(mode: &str, fixture: &Source, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&fixture.0)
        .args(args)
        .output()
        .expect("invoke real Cranelift CLI")
}

#[test]
fn inline_scalar_guest_is_sequential_and_emits_real_clif() {
    for (source, args, expected) in [
        (
            "fnc main():int { do sync task {}; return 42; }",
            &[][..],
            "42\n",
        ),
        (
            "fnc main():int { do sync task<void> { return; }; return sync task<int> { return 6*7; }; }",
            &[],
            "42\n",
        ),
        (
            "fnc main(v:int):int { return sync task<int> { return v + 2; }; }",
            &["40"][..],
            "42\n",
        ),
        (
            "fnc main():int { return sync task { return sync task<int> { return 42; }; }; }",
            &[],
            "42\n",
        ),
    ] {
        let fixture = Source::new(source);
        let checked = call("check", &fixture, &[]);
        assert!(
            checked.status.success(),
            "check: {}",
            String::from_utf8_lossy(&checked.stderr)
        );
        assert!(checked.stdout.is_empty());
        let ir = call("ir", &fixture, &[]);
        assert!(
            ir.status.success(),
            "CLIF: {}",
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));
        let native = call("run", &fixture, args);
        assert!(
            native.status.success(),
            "guest: {}",
            String::from_utf8_lossy(&native.stderr)
        );
        assert_eq!(native.stdout, expected.as_bytes(), "guest {source}");
    }
}

#[test]
fn inline_void_prefix_does_not_mask_checked_value_failure() {
    let fixture = Source::new(
        "fnc main():int { do sync task {}; return sync task<int> { return Integer.rem(5,0); }; }",
    );
    let checked = call("check", &fixture, &[]);
    assert!(
        checked.status.success(),
        "parse must admit the checked scalar"
    );
    let native = call("run", &fixture, &[]);
    assert_eq!(native.status.code(), Some(1));
    assert!(
        native.stdout.is_empty(),
        "checked failure must not publish a result"
    );
    assert!(String::from_utf8_lossy(&native.stderr).contains("integer divisor zero"));
}

#[test]
fn unverified_spawn_suspend_and_payload_forms_produce_no_guest_artifacts() {
    for source in [
        "fnc main():int { return task<int> { return 42; }; }",
        "fnc main():int { do task<int> { return 42; } return 0; }",
        "fnc main():int { return await task<int> { return 42; }; }",
        "fnc main():int { sync task {}; return 42; }",
        "fnc main():int { sync task<void> {}; return 42; }",
        "fnc main():int { do sync task<int> {}; return 0; }",
        "fnc main():int { do sync task<int> { return 42; }; return 0; }",
        "fnc main():int { do sync task { return 42; }; return 0; }",
        "fnc main():int { do sync task<void> { return 42; }; return 0; }",
        "fnc main():int { do sync task { do sync task<int> { return 42; }; }; return 0; }",
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
            assert!(
                got.stdout.is_empty(),
                "{mode} emitted guest artifact for {source}"
            );
            assert!(!got.stderr.is_empty(), "{mode} rejected without diagnostic");
        }
    }
}

#[test]
fn do_inline_void_only_diagnostics_reject_before_clif_or_execution() {
    for (source, expected) in [
        (
            "fnc main():int { do sync task<int> {}; return 42; }",
            "E_TASK_RETURN_TYPE",
        ),
        (
            "fnc main():int { do sync task<int> { return 42; }; return 0; }",
            "E_TASK_RETURN_TYPE",
        ),
        (
            "fnc main():int { do sync task { return 42; }; return 0; }",
            "E_TASK_RETURN_TYPE",
        ),
        (
            "fnc main():int { do sync task<void> { return 42; }; return 0; }",
            "E_TASK_RETURN_TYPE",
        ),
        ("fnc main():int { sync task {}; return 42; }", "E_SYNTAX"),
        (
            "fnc main():int { sync task<void> {}; return 42; }",
            "E_SYNTAX",
        ),
    ] {
        let fixture = Source::new(source);
        for mode in ["check", "ir", "run"] {
            let got = call(mode, &fixture, &[]);
            assert_eq!(got.status.code(), Some(1), "{mode} admitted {source}");
            assert!(
                got.stdout.is_empty(),
                "{mode} published stdout for {source}"
            );
            assert!(
                String::from_utf8_lossy(&got.stderr).contains(expected),
                "{mode} missing {expected} for {source}: {}",
                String::from_utf8_lossy(&got.stderr)
            );
        }
    }
}

#[test]
fn immutable_task_locals_use_scoped_native_ssa_slots() {
    for (source, args) in [
        ("fnc main():int {return sync task<int> {val int n=40;return n+2;};}", &[][..]),
        ("fnc main(v:int):int {val x=v+1;return sync task<int> {val y=x+1;return y;};}", &["40"][..]),
        ("fnc main():int {return sync task<int> {val x=40;return sync task<int> {val y=x+2;return y;};};}", &[]),
        ("fnc main():int {do sync task {val x=42;return;};return 42;}", &[]),
    ] {
        let fixture = Source::new(source);
        let checked = call("check", &fixture, &[]);
        assert!(checked.status.success(), "{source}: {}", String::from_utf8_lossy(&checked.stderr));
        let ir = call("ir", &fixture, &[]);
        assert!(ir.status.success(), "{source}: {}", String::from_utf8_lossy(&ir.stderr));
        assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));
        let native = call("run", &fixture, args);
        assert!(native.status.success(), "{source}: {}", String::from_utf8_lossy(&native.stderr));
        assert_eq!(native.stdout, b"42\n", "{source}");
    }
    for source in [
        "fnc main():int {do sync task {val x=42;};return x;}",
        "fnc main():int {return sync task<int> {val x=x;return x;};}",
        "fnc main():int {return sync task<int> {val x=1;val x=42;return x;};}",
        "fnc main():int {return sync task<int> {val x=1;x=42;return x;};}",
        "fnc main():int {do sync task {val x=1;return x;};return 42;}",
    ] {
        let fixture = Source::new(source);
        for mode in ["check", "ir", "run"] {
            let got = call(mode, &fixture, &[]);
            assert_eq!(got.status.code(), Some(1), "{mode}: {source}");
            assert!(got.stdout.is_empty(), "{mode}: {source}");
        }
    }
}
#[test]
fn unused_task_initializer_is_still_evaluated_once() {
    let fixture = Source::new("fnc main():int {return sync task<int> {val x=9223372036854775807+1;return 42;};}");
    assert!(call("check", &fixture, &[]).status.success());
    let got = call("run", &fixture, &[]);
    assert_eq!(got.status.code(), Some(1));
    assert!(got.stdout.is_empty());
    assert!(String::from_utf8_lossy(&got.stderr).contains("overflow"));
}
