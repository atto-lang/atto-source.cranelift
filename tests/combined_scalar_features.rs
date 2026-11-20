//! Cross-feature regression: local constexpr/val, sequence and the independently
//! admitted pure-pipeline frontend must share a verified native JIT boundary.
//! These are *authored* tests until executed on this exact commit.
use std::io::Write;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn execute(mode: &str, source: &str, args: &[&str]) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-integration-{}-{id}.atto",
        std::process::id()
    ));
    let mut fixture = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("exclusive guest fixture; never follow an existing symlink");
    fixture.write_all(source.as_bytes()).expect("write guest bytes");
    drop(fixture);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .expect("real native Cranelift CLI");
    std::fs::remove_file(&path).expect("remove guest fixture");
    output
}

fn admitted(source: &str, args: &[&str], expected: &[u8]) {
    for mode in ["check", "ir", "run"] {
        let runtime_args = if mode == "run" { args } else { &[][..] };
        let output = execute(mode, source, runtime_args);
        assert_eq!(output.status.code(), Some(0),
            "{mode}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stderr.is_empty(), "{mode}: unexpected diagnostic");
        if mode == "run" {
            assert_eq!(output.stdout, expected, "{mode}: guest value changed");
        } else if mode == "ir" {
            assert!(String::from_utf8_lossy(&output.stdout).contains("function"),
                "IR mode must return real checked Cranelift IR");
        } else {
            assert!(output.stdout.is_empty(), "check must not run or print guest");
        }
    }
}

fn rejected(source: &str, expected_diag: &str) {
    for mode in ["check", "ir", "run"] {
        let output = execute(mode, source, &[]);
        assert_eq!(output.status.code(), Some(1), "{mode}: rejected source succeeded");
        assert!(output.stdout.is_empty(), "{mode}: rejected source published IR/guest output");
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected_diag),
            "{mode}: expected {expected_diag}; got {}", String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn constant_seq_and_runtime_val_share_scalar_native_backend() {
    admitted(
        "fnc main(a:int):int{val int live=a+1;const expected: int=seq(1,40+2);return seq(live,expected);}",
        &["40"], b"42\n",
    );
    admitted(
        "fnc main():int{int answer=(> 1,2,42);val checked=seq(answer-1,answer);return checked;}",
        &[], b"42\n",
    );
    admitted(
        "fnc main(a:int):int{val first=seq(1,a+2);val second=seq(first,42);return second;}",
        &["40"], b"42\n",
    );
}

#[test]
fn static_const_seq_checks_every_discarded_element_before_ir() {
    rejected("fnc main():int{const x=seq(9223372036854775807+1,42);return x;}",
             "E_CONST_OVERFLOW");
    rejected("fnc main(a:int):int{const x=seq(a+1,42);return x;}",
             "E_NON_CONST_INITIALIZER");
    rejected("fnc main():int{const x=seq(Integer.rem(4,0),42);return x;}",
             "E_NON_CONST_INITIALIZER");
}

#[test]
fn unobserved_runtime_initializer_seq_must_not_erase_traps() {
    let source = "fnc main(a:int):int{val ignored=seq(a+1,42);return 42;}";
    let ir = execute("ir", source, &[]);
    assert_eq!(ir.status.code(), Some(0), "runtime initializer should admit IR");
    assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));

    let safe = execute("run", source, &["1"]);
    assert_eq!(safe.status.code(), Some(0));
    assert_eq!(safe.stdout, b"42\n");

    let trapped = execute("run", source, &["9223372036854775807"]);
    assert_eq!(trapped.status.code(), Some(1), "discarded initializer overflow");
    assert!(trapped.stdout.is_empty(), "no result published after a trap");
    assert!(String::from_utf8_lossy(&trapped.stderr).contains("overflow"));
}

#[test]
fn pipeline_frontend_remains_native_and_mixed_profiles_fail_closed() {
    let pipelines = include_str!("fixtures/functional-pipelines-v1/inline-arrow-body.atto");
    admitted(pipelines, &[], b"83\n");

    // A single-guest mixed pipeline + sequence requires a shared typed IR.
    // Until then it may not be guessed as a pure function or host-evaluated.
    let mixed = "fnc double(int x):int{return x*2;} fnc main():int{return seq(1,21) |> double;}";
    for mode in ["check", "ir", "run"] {
        let output = execute(mode, mixed, &[]);
        assert_eq!(output.status.code(), Some(1), "{mode}: unsupported mix admitted");
        assert!(output.stdout.is_empty(), "{mode}: unverified mixed source published output");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "));
    }
}
