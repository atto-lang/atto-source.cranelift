//! This profile is intentionally reject-only until typed protected-state lowering exists.
use std::{fs,process::Command};
#[test]
fn shared_positive_state_programs_fail_closed_without_ir_or_execution() {
    let dir=std::env::temp_dir().join(format!("atto-state-denial-{}",std::process::id()));
    fs::create_dir(&dir).unwrap();
    for (i,source) in [
        include_str!("fixtures/state-intrinsics-v1/pass/state-short-intrinsics.atto"),
        include_str!("fixtures/state-intrinsics-v1/pass/state-owned-tuple-snapshot.atto"),
        include_str!("fixtures/state-intrinsics-v1/pass/state-write-expressions.atto")
    ].iter().enumerate(){
        let input=dir.join(format!("case-{i}.atto"));
        fs::write(&input,source).unwrap();
        for mode in ["run","ir"] {
            let result=Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg(mode).arg(&input).output().unwrap();
            assert_eq!(result.status.code(),Some(1));
            assert!(result.stdout.is_empty(),"unexpected artifact or guest output");
            let err=String::from_utf8_lossy(&result.stderr);
            assert!(err.contains("DECL_KIND_UNSUPPORTED: state container"),"{err}");
        }
    }
    let control=dir.join("control.atto");
    fs::write(&control,"fnc main():int{return 42;}").unwrap();
    let result=Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("run").arg(&control).output().unwrap();
    assert!(result.status.success(),"{:?}",result.stderr);
    assert_eq!(result.stdout,b"42\n");
    fs::remove_dir_all(dir).unwrap();
}
