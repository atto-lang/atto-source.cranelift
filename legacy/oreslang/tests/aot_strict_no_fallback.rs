//! Source-repository native CLI regression for unavailable AOT modes.
//! An unimplemented AOT backend must never claim success through JIT fallback.
use std::process::Command;

#[test]
fn unsupported_aot_modes_fail_before_source_access_or_jit() {
    let bin = env!("CARGO_BIN_EXE_orescranelift");
    for mode in ["aot", "full-aot"] {
        let out = Command::new(bin)
            .arg(mode)
            .arg("this-source-must-not-be-opened.atto")
            .arg("this-output-must-not-be-written.o")
            .output()
            .expect("launch actual native Cranelift CLI");
        assert_eq!(out.status.code(), Some(1), "{mode} must be rejected");
        assert!(out.stdout.is_empty(), "{mode} must not print a guest result");
        let diagnostic = String::from_utf8_lossy(&out.stderr);
        assert!(
            diagnostic.starts_with("orescranelift: ")
                && diagnostic.contains("full AOT object/executable emission is not implemented")
                && diagnostic.contains("refusing JIT fallback"),
            "{mode} must return a compiler-owned AOT denial: {diagnostic}"
        );
    }
}

#[test]
fn object_mode_is_not_misrepresented_as_supported_aot() {
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("object")
        .arg("this-source-must-not-be-opened.atto")
        .arg("this-output-must-not-be-written.o")
        .output()
        .expect("launch native CLI for unsupported object mode");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage: orescranelift"),
        "unknown object command must fail closed"
    );
}
