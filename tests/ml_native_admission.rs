//! Compiler CLI acceptance for the *current* stage-0 ML arithmetic slice.
//! This tests the real Oreslang -> Cranelift JIT only when Cargo runs.
//! It does not claim FP64, tensor, autograd, actor, or other backend parity.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

// Keep file-backed child output outside OS pipes: try_wait() does not drain
// pipes and verbose IR/stderr could otherwise fill a pipe and deadlock.
// Files are created exclusively, mode 0600 on Unix, and removed on panics.
struct ScratchFiles(Vec<PathBuf>);
impl Drop for ScratchFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn exclusive_file(path: &Path, guard: &mut ScratchFiles) -> File {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path).expect("exclusive temporary fixture/output creation");
    guard.0.push(path.to_path_buf());
    file
}

fn execute(source: &str, mode: &str) -> Output {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "orescranelift-ml-admission-{}-{nonce}.ores",
        std::process::id()
    ));
    // The unique source/output filenames must not overwrite existing files
    // or symlinks, including during simultaneous Cargo test invocations.
    let mut cleanup = ScratchFiles(Vec::new());
    let mut file = exclusive_file(&path, &mut cleanup);
    file.write_all(source.as_bytes()).expect("write fixture");
    file.sync_all().expect("sync fixture");
    drop(file);

    let stdout_path = path.with_extension("stdout");
    let stderr_path = path.with_extension("stderr");
    let stdout = exclusive_file(&stdout_path, &mut cleanup);
    let stderr = exclusive_file(&stderr_path, &mut cleanup);
    let mut child = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("invoke actual Cranelift compiler CLI");
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll compiler subprocess") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Cranelift {mode} exceeded five-second native admission guard");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // No pipe read deadlock: the child wrote to bounded-lifetime regular files.
    // Drop cleans up even if a read/assertion panics.
    Output {
        status,
        stdout: std::fs::read(&stdout_path).expect("read native stdout"),
        stderr: std::fs::read(&stderr_path).expect("read native stderr"),
    }
}

#[test]
fn scalar_gradient_reference_runs_through_native_jit() {
    // A deliberately scalar training-gradient expression:
    // x=3, w=5, b=1, target=9; d/dw (wx+b-target)^2 = 2*(16-9)*3 = 42.
    // This is NOT a tensor gradient or autodiff runtime.
    let source = "fnc main():int{return 2*((5*3+1)-9)*3;}";
    let result = execute(source, "run");
    assert!(
        result.status.success(),
        "native scalar gradient rejected: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"42\n");
    assert!(result.stderr.is_empty());
}

#[test]
fn integer_training_primitives_compile_to_ir_and_execute() {
    // Portable integer-only subset of the OresLLM scalar fixture; these are
    // source-to-real-JIT assertions, NOT FP64 or autograd evidence.
    let cases = [
        ("gradient", "fnc main():int{return 2*((5*3+1)-9)*3;}", "42\n"),
        ("associativity", "fnc main():int{return (7*8)-14;}", "42\n"),
        ("signed_subtraction", "fnc main():int{return 13-(-29);}", "42\n"),
        ("zero_cancellation", "fnc main():int{return (3*14)+(0*7);}", "42\n"),
    ];
    for (name, source, expected) in cases {
        let ir = execute(source, "ir");
        assert_eq!(ir.status.code(), Some(0), "{name}: compiler IR phase rejected source");
        assert!(!ir.stdout.is_empty(), "{name}: empty compiler/JIT IR");
        assert!(ir.stderr.is_empty(), "{name}: unexpected compiler IR diagnostics");
        let run = execute(source, "run");
        assert_eq!(run.status.code(), Some(0), "{name}: native JIT failed");
        assert_eq!(run.stdout, expected.as_bytes(), "{name}: wrong real native value");
        assert!(run.stderr.is_empty(), "{name}: unexpected native diagnostics");
    }
}

#[test]
fn unsupported_ml_features_fail_before_native_ir_or_execution() {
    let rejected = [
        ("float literal", "fnc main():int{return 1.5;}"),
        ("float result", "fnc main():float{return 1.0;}"),
        ("tensor constructor", "fnc main():int{return TensorF64(1);}"),
        ("tensor import", "import fnc add from './tensor'; fnc main():int{return 42;}"),
        ("ML class", "define class TensorF64 as end fnc main():int{return 42;}"),
        ("autograd helper", "fnc gradient(x:int):int{return x;} fnc main():int{return gradient(2);}"),
        ("ownership", "fnc main():int{return rt borrow 42;}"),
        ("actor source", "define actor Trainer as end fnc main():int{return 42;}"),
        ("async source", "pub async fnc main():int{return 42;}"),
    ];
    for (name, source) in rejected {
        for mode in ["ir", "run"] {
            let result = execute(source, mode);
            assert_eq!(
                result.status.code(),
                Some(1),
                "{name} {mode}: unsupported source must be a compiler rejection"
            );
            assert!(
                result.stdout.is_empty(),
                "{name} {mode}: unsupported source emitted native IR or a guest result"
            );
            let diagnostics = String::from_utf8_lossy(&result.stderr);
            assert!(
                !diagnostics.trim().is_empty(),
                "{name} {mode}: missing compiler rejection diagnostic"
            );
            assert!(
                !diagnostics.contains("panicked at"),
                "{name} {mode}: compiler panicked: {diagnostics}"
            );
        }
    }
}

#[test]
fn checked_overflow_occurs_in_native_execution_not_ir_acceptance() {
    let source = "fnc main():int{return (9223372036854775807+1)*0;}";
    let ir = execute(source, "ir");
    assert!(
        ir.status.success(),
        "valid source must lower to native IR before runtime overflow: {}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(
        !ir.stdout.is_empty(),
        "IR emission must produce real Cranelift IR, not silently skip"
    );
    let runtime = execute(source, "run");
    assert!(!runtime.status.success(), "checked overflow was accepted");
    assert!(runtime.stdout.is_empty(), "overflow published a native result");
    assert!(!runtime.stderr.is_empty(), "overflow needs a diagnostic");
}
