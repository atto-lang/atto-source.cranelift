//! Compiler CLI acceptance for the *current* stage-0 ML arithmetic slice.
//! This tests the real Oreslang -> Cranelift JIT only when Cargo runs.
//! It does not claim FP64, tensor, autograd, actor, or other backend parity.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn execute(source: &str, mode: &str) -> Output {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "orescranelift-ml-admission-{}-{nonce}.ores",
        std::process::id()
    ));
    // Never overwrite a file or symlink pre-existing at the chosen name.
    // An unexpected collision is a test failure, not a reason to truncate.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("exclusive temp fixture creation");
    file.write_all(source.as_bytes()).expect("write fixture");
    file.sync_all().expect("sync fixture");
    drop(file);
    let mut child = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("invoke actual Cranelift compiler CLI");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().expect("poll compiler subprocess").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill hung compiler subprocess");
            let _ = child.wait();
            std::fs::remove_file(&path).expect("delete timed-out fixture");
            panic!("Cranelift {mode} exceeded five-second native admission guard");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = child.wait_with_output().expect("collect compiler output");
    std::fs::remove_file(&path).expect("delete temp fixture");
    result
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
