use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(1);

// Actual orescranelift invocation. These test its native machine-code JIT,
// never evaluate the source with a host-language expression interpreter.
fn execute(source: &str, arguments: &[&str]) -> Output {
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "orescranelift-stategraph-parity-{}-{}.ores",
        std::process::id(),
        serial
    ));
    std::fs::write(&file, source).expect("write Oreslang fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&file)
        .args(arguments)
        .output()
        .expect("execute actual Oreslang Cranelift compiler");
    std::fs::remove_file(file).expect("remove temporary source");
    result
}

fn accepted(source: &str, arguments: &[&str], expected: &str) {
    let out = execute(source, arguments);
    assert!(
        out.status.success(),
        "native scalar guest failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

fn unsupported(source: &str) {
    let out = execute(source, &[]);
    assert!(
        !out.status.success(),
        "unimplemented StateGraph construct silently accepted"
    );
    assert!(out.stdout.is_empty(), "failed source emitted guest output");
    assert!(!out.stderr.is_empty(), "rejection must have a diagnostic");
}

#[test]
fn stategraph_common_checked_scalar_native_jit() {
    accepted(
        "fnc main(steps:int,delta:int):int{return steps+delta*2;}",
        &["6", "18"],
        "42\n",
    );
    accepted(
        "fnc main(steps:int,delta:int):int{return steps+delta*2;}",
        &["0", "0"],
        "0\n",
    );
    accepted(
        "fnc main(steps:int,delta:int):int{return steps+delta*2;}",
        &["-2", "2"],
        "2\n",
    );
    let overflow = execute(
        "fnc main(steps:int,delta:int):int{return steps+delta*2;}",
        &["0", "9223372036854775807"],
    );
    assert!(!overflow.status.success(), "overflow silently wrapped");
    assert!(overflow.stdout.is_empty(), "overflow printed a false result");
    assert!(!overflow.stderr.is_empty(), "overflow needs a diagnostic");
}

#[test]
fn stategraph_unimplemented_features_are_explicitly_rejected() {
    // This is negative evidence, NOT support for the full StateGraph library.
    // Each rejection identifies native grammar/lowering work before parity.
    let blockers = [
        "define module sg as pub fnc increment(int x):int{return x+1;} end",
        "define pub class State as pub val int steps; end",
        "fnc main():int{let mut Array<int> xs = new Array<int>();return 1;}",
        "fnc main():int{return Some(42).unwrap();}",
        "fnc main():int{val Result<int,String> x = Ok(42);return x.unwrap();}",
        "import module stategraph from \"./stategraph\";fnc main():int{return 42;}",
        "define isolated actor Coordinator as state { steps:int=0; } end",
        "fnc increment(int x):int{return x+1;} fnc main():int{return increment(41);}",
    ];
    for source in blockers {
        unsupported(source);
    }
}
