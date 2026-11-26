//! This profile is intentionally reject-only until typed protected-state lowering exists.
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn shared_positive_state_programs_fail_closed_without_ir_or_execution() {
    let path = std::env::temp_dir().join(format!(
        "atto-state-denial-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    let dir = Temp(path);
    for (i, source) in [
        include_str!("fixtures/state-intrinsics-v1/pass/state-short-intrinsics.atto"),
        include_str!("fixtures/state-intrinsics-v1/pass/state-owned-tuple-snapshot.atto"),
        include_str!("fixtures/state-intrinsics-v1/pass/state-write-expressions.atto"),
    ]
    .iter()
    .enumerate()
    {
        let input = dir.0.join(format!("case-{i}.atto"));
        fs::write(&input, source).unwrap();
        for mode in ["run", "ir"] {
            let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&input)
                .current_dir(&dir.0)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(1), "{i} {mode}: {result:?}");
            assert!(
                result.stdout.is_empty(),
                "unexpected artifact or guest output"
            );
            let err = String::from_utf8_lossy(&result.stderr);
            assert!(
                err.contains("DECL_KIND_UNSUPPORTED: state container"),
                "{err}"
            );
        }
        assert_eq!(fs::read_to_string(&input).unwrap(), *source);
    }
    let control = dir.0.join("control.atto");
    fs::write(&control, "fnc main():int{return 42;}").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&control)
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(result.stdout, b"42\n");
}
