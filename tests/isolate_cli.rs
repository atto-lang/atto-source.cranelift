use std::process::Command;

#[test]
fn trusted_isolate_cli_keeps_tenant_scalar_states_separate() {
    let path =
        std::env::temp_dir().join(format!("orescranelift-isolate-{}.atto", std::process::id()));
    std::fs::write(&path, "fnc main(old:int,delta:int):int{return old+delta;}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .args(["isolate"])
        .arg(&path)
        .args(["41", "-7"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"41 -7\n");
    assert!(output.stderr.is_empty());
    let rejected = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .args(["isolate"])
        .arg(&path)
        .arg("41")
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(rejected.status.code(), Some(1));
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("isolate mode expects"));
}
