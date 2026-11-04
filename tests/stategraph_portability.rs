use std::io::Write;
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
    let mut source_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .expect("create fresh Oreslang fixture without following a symlink");
    source_file
        .write_all(source.as_bytes())
        .expect("write Oreslang fixture");
    drop(source_file);
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
    assert_eq!(
        out.status.code(),
        Some(1),
        "unsupported source must be rejected by the compiler with exit code 1"
    );
    assert!(
        out.stdout.is_empty(),
        "rejected source emitted guest output"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("orescranelift: "),
        "rejection must originate from the Oreslang compiler"
    );
}

#[test]
fn stategraph_common_checked_scalar_native_jit() {
    let shared = include_str!("../contracts/stategraph-portability/shared-scalar.ores");
    assert_eq!(shared, "fnc main():int{return 6*7;}\n");
    accepted(shared, &[], "42\n");
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
    assert!(
        overflow.stdout.is_empty(),
        "overflow printed a false result"
    );
    assert!(!overflow.stderr.is_empty(), "overflow needs a diagnostic");
}

#[test]
fn stategraph_unimplemented_features_are_explicitly_rejected() {
    // This is negative evidence, NOT support for the full StateGraph library.
    // Each rejection identifies native grammar/lowering work before parity.
    // The exact canonical GraphState<T> userland source is a required
    // compatibility blocker until generic class lowering actually exists.
    let generic = include_str!("../contracts/stategraph-portability/shared-generic-state.ores");
    assert_eq!(
        generic,
        concat!(
            "// Nominal generic graph state, owned immutable value.\n",
            "define pub class GraphState<T> as\n",
            "  pub val T value;\n",
            "  pub constructor(T value) {\n",
            "    self.value = value;\n",
            "  }\n",
            "end\n\n",
            "pub fnc main(): int {\n",
            "  val GraphState<int> initial = new GraphState<int>(42);\n",
            "  return initial.value;\n",
            "}\n"
        )
    );
    unsupported(generic);

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
