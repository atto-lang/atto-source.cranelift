//! Native Oreslang assertion subset for scalar-only toolchains.
//! This compiles *actual* .ores source through the Cranelift CLI.
//! It is not evidence that Cranelift supports ores.std/testing classes/imports.
use std::path::{Path, PathBuf};
use std::process::Command;

const DIR: &str = "contracts/parity/native-scalar-zero-v1";
const POSITIVE: [(&str, &str); 5] = [
    (
        "precedence-zero.ores",
        "pub fnc main():int{return (2 + 3 * 4) - 14;}\n",
    ),
    (
        "i64-precision-zero.ores",
        "pub fnc main():int{return (9007199254740993 - 9007199254740992) - 1;}\n",
    ),
    (
        "signed-opposite-zero.ores",
        "pub fnc main():int{return (9007199254740993 + (-9007199254740992)) - 1;}\n",
    ),
    (
        "nested-subtraction-zero.ores",
        "pub fnc main():int{return (100 - (50 - (20 - 7))) - 63;}\n",
    ),
    (
        "min-boundary-zero.ores",
        "pub fnc main():int{return (-9223372036854775808 + 1) - (-9223372036854775807);}\n",
    ),
];
const NEGATIVE: (&str, &str) = (
    "reject-removed-operator.ores",
    "pub fnc main():int{return 7 % 3;}\n",
);

fn source(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(DIR).join(name)
}
fn run(path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(path)
        .output()
        .expect("real Cranelift compiler/guest runner must execute")
}

#[test]
fn native_fixture_bytes_are_pinned_and_complete() {
    for (name, expected) in POSITIVE.into_iter().chain([NEGATIVE]) {
        let bytes = std::fs::read(source(name)).expect("missing reviewed Oreslang fixture");
        assert_eq!(
            bytes,
            expected.as_bytes(),
            "unreviewed source drift: {name}"
        );
    }
}

#[test]
fn five_native_oreslang_assertions_compile_and_return_zero() {
    for (name, _) in POSITIVE {
        let output = run(&source(name));
        assert!(
            output.status.success(),
            "actual Cranelift compiler/guest rejected {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.stdout, b"0\n",
            "{name}: nonzero assertion or forged output"
        );
        assert!(output.stderr.is_empty(), "{name}: unexplained guest stderr");
    }
}

#[test]
fn unsupported_percent_is_compiler_rejected_not_a_false_pass() {
    let output = run(&source(NEGATIVE.0));
    assert!(
        !output.status.success(),
        "removed percent operator was compiled"
    );
    assert!(
        output.stdout.is_empty(),
        "rejected guest produced value output"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("orescranelift:"),
        "missing compiler-owned rejection diagnostic"
    );
}

#[test]
fn deliberately_wrong_assertion_never_emits_zero() {
    let work = std::env::temp_dir().join(format!(
        "orescranelift-negative-assertion-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir(&work).expect("create isolated mutant workspace");
    let path = work.join("wrong-assertion.ores");
    std::fs::write(&path, "pub fnc main():int{return (2 + 3 * 4) - 13;}\n").expect("write mutant");
    let output = run(&path);
    let _ = std::fs::remove_dir_all(&work);
    assert!(
        output.status.success(),
        "mutant must compile as a valid program"
    );
    assert_eq!(
        output.stdout, b"1\n",
        "mutated expected value must be detected"
    );
    assert_ne!(
        output.stdout, b"0\n",
        "mutant must never pass the zero assertion"
    );
}
