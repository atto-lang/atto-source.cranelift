use std::process::Command;

#[test]
fn uhl1_capabilities_are_explicitly_unsupported_for_scalar_cranelift() {
    let executable = env!("CARGO_BIN_EXE_orescranelift");
    let output = Command::new(executable)
        .arg("hotload-capabilities")
        .output()
        .expect("run native Cranelift capabilities CLI");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("canonical ASCII capability response");
    assert!(stdout.ends_with("\n"));
    assert!(stdout.contains("\"interface\":\"ores.hotload/v1\""));
    assert!(stdout.contains("\"backend\":\"cranelift\""));
    assert!(stdout.contains("\"features\":0"));
    assert!(stdout.contains("\"executionModes\":0"));
    assert!(stdout.contains("\"guestControlsHotLoad\":false"));
    assert!(stdout.contains("\"productionActorHotLoad\":false"));
    assert!(stdout.contains("\"maxCheckpointBytes\":0"));
}

#[test]
fn uhl1_cannot_take_source_arguments_or_fake_hotload_subcommands() {
    let executable = env!("CARGO_BIN_EXE_orescranelift");
    for args in [
        ["hotload-capabilities", "forged-manifest"],
        ["hotload-capabilities", "actor.ores"],
        ["hotload-stage", "actor.ores"],
        ["hotload-commit", "7"],
    ] {
        let output = Command::new(executable)
            .args(args)
            .output()
            .expect("run denied Cranelift hotload CLI");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty(), "denied operation emitted stdout");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("hotload-capabilities") || stderr.contains("usage: orescranelift"),
            "unexpected failure diagnostic: {stderr}"
        );
        assert!(!stderr.contains("panic"), "bad hot load request panicked");
    }
}

#[test]
fn capability_query_does_not_require_file_or_trigger_native_guest_jit() {
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("hotload-capabilities")
        .current_dir(std::env::temp_dir())
        .output()
        .expect("capabilities must run independent of source file");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"features\":0"));
}
