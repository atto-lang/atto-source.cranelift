//! Exact .atto guest compiler failure/zero-artifact admission.
//! These are UNSUPPORTED target fixtures, not passed Result semantics.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(1);

fn compile(mode: &str, source: &[u8]) -> std::process::Output {
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-result-{}-{id}.atto",
        std::process::id()
    ));
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).expect("exclusive fixture");
    file.write_all(source).expect("write exact source fixture");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("invoke compiler");
    std::fs::remove_file(path).expect("remove source fixture");
    output
}
#[test]
fn result_question_trap_match_fail_closed_before_jit_or_clif() {
    const CASES: &[(&str, &[u8])] = &[
        (
            "parse_id",
            include_bytes!("fixtures/result-int-parse-v1/parse_id.atto"),
        ),
        (
            "parse_builtin",
            include_bytes!("fixtures/result-int-parse-v1/parse_builtin.atto"),
        ),
        (
            "question",
            include_bytes!("fixtures/result-int-parse-v1/question.atto"),
        ),
        (
            "trap",
            include_bytes!("fixtures/result-int-parse-v1/trap.atto"),
        ),
        (
            "match",
            include_bytes!("fixtures/result-int-parse-v1/match.atto"),
        ),
    ];
    for &(case_name, source) in CASES {
        for mode in ["check", "run", "ir"] {
            let output = compile(mode, source);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{case_name}/{mode}: unsupported syntax unexpectedly executed"
            );
            assert!(
                output.stdout.is_empty(),
                "{case_name}/{mode}: unexpected IR/JIT result artifact"
            );
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(
                err.starts_with("orescranelift: ") && !err.contains("panicked"),
                "{case_name}/{mode}: missing sanitized compiler diagnostic: {err}"
            );
        }
    }
}
#[test]
fn scalar_control_remains_real_cranelift_guest() {
    let output = compile("run", b"fnc main():int{return 42;}\n");
    assert!(output.status.success(), "scalar source failed admission");
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn checked_unwrap_remains_denied_in_all_scalar_artifact_modes() {
    let cases = [
        "fnc main():int{val Option<int> x=None;return x.unwrap();}",
        "fnc main():int{val Option<int> x=Some(42);return x.unwrap();}",
        "fnc main():int{val Result<int,Error> x=Err(Error(\"bad\"));return x.unwrap();}",
        "fnc main():int{val Result<int,Error> x=trap! None.unwrap();return 42;}",
        "pub fnc main():int{val Option<int> x=None;return trap! x.unwrap() |> match<int> over on Ok(n):int -> n, on Err():int -> 42, end;}",
    ];
    for source in cases {
        for mode in ["check", "run", "ir", "emit-object", "emit-object-strict"] {
            let output = compile(mode, source.as_bytes());
            assert_eq!(output.status.code(), Some(1), "{mode}: {source}");
            assert!(output.stdout.is_empty(), "unexpected artifact {mode}: {source}");
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(err.starts_with("orescranelift: ") && !err.contains("panicked"),
                "unsanitized diagnostic {mode}: {err}");
        }
    }
}
