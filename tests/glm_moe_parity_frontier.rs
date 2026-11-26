//! Honest GLM/MoE cross-backend frontier for the stage-0 Cranelift JIT.
//! These are executable positive and explicit unsupported-source tests.
//! They do not prove that the shared 20-program GLM corpus compiles here.
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
const COMPILER_DEADLINE: Duration = Duration::from_secs(5);
const MAX_CAPTURE: usize = 65536;

// Always drain both pipes while the compiler is alive. Waiting for process
// exit before draining can deadlock on diagnostics larger than pipe capacity.
fn drain_bounded(mut input: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut stored = Vec::new();
    let mut exceeded = false;
    let mut buf = [0u8; 8192];
    loop {
        let len = input.read(&mut buf)?;
        if len == 0 {
            return Ok((stored, exceeded));
        }
        let remaining = MAX_CAPTURE.saturating_sub(stored.len());
        stored.extend_from_slice(&buf[..len.min(remaining)]);
        if len > remaining {
            exceeded = true;
        }
    }
}

struct GuestFixture(PathBuf);

impl Drop for GuestFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run_guest(mode: &str, source: &str, args: &[&str]) -> Output {
    assert!(
        matches!(mode, "run" | "ir"),
        "only actual compiler modes allowed"
    );
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-cranelift-glm-{}-{sequence}.ores",
        std::process::id()
    ));
    // create_new never overwrites a predictable preexisting name or symlink.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&path).unwrap();
    // Only an actually created file may be owned and unlinked on Drop.
    // A failed exclusive create must never unlink someone else's path.
    let fixture = GuestFixture(path);
    file.write_all(source.as_bytes()).unwrap();
    file.sync_all().unwrap();
    drop(file);

    // Avoid indefinite CI hangs on compiler deadlocks or runaway JITs.
    // Each pipe is drained concurrently with strictly bounded storage.
    let mut child = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&fixture.0)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout_pipe = child.stdout.take().expect("captured stdout");
    let stderr_pipe = child.stderr.take().expect("captured stderr");
    let stdout_thread = std::thread::spawn(move || drain_bounded(stdout_pipe));
    let stderr_thread = std::thread::spawn(move || drain_bounded(stderr_pipe));
    let deadline = Instant::now() + COMPILER_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native compiler exceeded five-second per-case deadline");
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native compiler child status failed: {error}");
            }
        }
    }
    let status = child.wait().expect("compiler termination status");
    let (stdout, stdout_exceeded) = stdout_thread
        .join()
        .expect("stdout reader panicked")
        .expect("stdout read failed");
    let (stderr, stderr_exceeded) = stderr_thread
        .join()
        .expect("stderr reader panicked")
        .expect("stderr read failed");
    assert!(
        !stdout_exceeded && !stderr_exceeded,
        "native compiler exceeded 64 KiB per-stream output budget"
    );
    Output {
        status,
        stdout,
        stderr,
    }
}

fn expect_native(source: &str, args: &[&str], expected: &str) {
    let result = run_guest("run", source, args);
    assert!(
        result.status.success(),
        "native source should succeed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), expected);
    assert!(
        result.stderr.is_empty(),
        "successful guest emitted diagnostics"
    );
}

fn expect_unsupported(source: &str) {
    for mode in ["run", "ir"] {
        let result = run_guest(mode, source, &[]);
        assert!(
            !result.status.success(),
            "unsupported GLM source admitted via {mode}"
        );
        assert!(
            result.stdout.is_empty(),
            "unsupported source printed output via {mode}"
        );
        assert_eq!(
            result.status.code(),
            Some(1),
            "rejection must be a compiler diagnostic"
        );
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        assert!(
            diagnostic.starts_with("orescranelift: "),
            "unsupported-source refusal must originate from Cranelift frontend"
        );
        assert!(
            !diagnostic.contains("panicked at") && !diagnostic.contains("stack backtrace"),
            "unsupported source crashed the compiler in {mode}"
        );
    }
}

#[test]
fn supported_scalar_produces_native_clif_in_ir_mode() {
    let output = run_guest("ir", "fnc main():int{return 40+2;}", &[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("function"),
        "positive source must lower into Cranelift IR"
    );
}

#[test]
fn scalar_top1_score_difference_intersection_executes_as_native_jit() {
    expect_native("fnc main(a:int,b:int):int{return a-b;}", &["9", "2"], "7\n");
    expect_native(
        "fnc main(a:int,b:int):int{return a-b;}",
        &["-5", "-1"],
        "-4\n",
    );
}

#[test]
fn scalar_assignment_total_arithmetic_intersection_executes_as_native_jit() {
    expect_native(
        "fnc main(a:int,b:int,tokens:int,k:int):int{return a+b-tokens*k;}",
        &["2", "2", "4", "1"],
        "0\n",
    );
    expect_native(
        "fnc main(a:int,b:int,tokens:int,k:int):int{return a+b-tokens*k;}",
        &["1", "1", "4", "1"],
        "-2\n",
    );
}

#[test]
fn scalar_learning_step_arithmetic_intersection_executes_as_native_jit() {
    expect_native("fnc main(prior:int):int{return prior+1;}", &["3"], "4\n");
    expect_native(
        "fnc main(prior:int):int{return prior+1;}",
        &["9223372036854775806"],
        "9223372036854775807\n",
    );
}

#[test]
fn checked_overflow_rejects_before_claiming_glm_guest_success() {
    let result = run_guest(
        "run",
        "fnc main(prior:int):int{return prior+1;}",
        &["9223372036854775807"],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!result.stderr.is_empty());
}

#[test]
fn helper_functions_in_shared_glm_corpus_are_not_supported() {
    expect_unsupported(
        "fnc route(a:int,b:int):int{return a-b;} pub fnc main():int{return route(9,2);}",
    );
}

#[test]
fn deterministic_top1_control_flow_is_not_yet_lowered() {
    expect_unsupported("pub fnc main():int{if (9>=2) {return 42;} fi return 1;}");
}

#[test]
fn boolean_capacity_and_cancellation_logic_must_reject() {
    expect_unsupported("pub fnc main():int{if (true && false) {return 1;} fi return 42;}");
}

#[test]
fn float_fixed_gate_prediction_must_not_silently_become_integer_arithmetic() {
    expect_unsupported("pub fnc main():int{if (0.75*2.0==1.5) {return 42;} fi return 1;}");
}

#[test]
fn actor_ownership_and_suspending_source_cannot_bypass_stage_zero() {
    expect_unsupported(
        "define isolated actor Gate as state int value; end pub fnc main():int{return 42;}",
    );
}

#[test]
fn cross_file_imports_are_not_flattened_into_an_unreviewed_program() {
    expect_unsupported(
        "import fnc route from \"./other.ores\"; pub fnc main():int{return route();}",
    );
}

#[test]
fn undocumented_power_operator_still_rejects() {
    expect_unsupported("pub fnc main():int{return 2 ^ 3;}");
}

#[test]
fn checked_integer_division_is_a_supported_scalar_intersection() {
    expect_native("pub fnc main():int{return 3 / 2;}", &[], "1\n");
    expect_native("pub fnc main():int{return -3 / 2;}", &[], "-1\n");
    let fault = run_guest("run", "pub fnc main():int{return 3 / 0;}", &[]);
    assert_eq!(fault.status.code(), Some(1));
    assert!(fault.stdout.is_empty());
    assert!(String::from_utf8_lossy(&fault.stderr).starts_with("orescranelift: "));
    let output = run_guest("ir", "pub fnc main():int{return 3 / 2;}", &[]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("function"));
}

#[test]
fn excessive_source_and_nul_bytes_are_bounded_frontend_rejections() {
    expect_unsupported(&format!(
        "fnc main():int{{return 42;}}//{}",
        "x".repeat(1024 * 1024)
    ));
    expect_unsupported("fnc main():int{return 42;}\u{0000}");
}

#[test]
fn oversized_diagnostic_capture_is_reported_as_failure_not_truncated_success() {
    let data = vec![b'X'; MAX_CAPTURE + 4096];
    let (captured, overflow) = drain_bounded(&data[..]).unwrap();
    assert_eq!(captured.len(), MAX_CAPTURE);
    assert!(overflow, "truncation must be explicit");
    let (captured, overflow) = drain_bounded(&data[..MAX_CAPTURE]).unwrap();
    assert_eq!(captured.len(), MAX_CAPTURE);
    assert!(!overflow, "exact capture limit remains admissible");
}
