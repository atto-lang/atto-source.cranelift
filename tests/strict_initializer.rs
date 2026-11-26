//! Actual strict object-mode compiler admission and output publication.
use std::{fs, process::Command};
#[test]
fn typed_locals_require_explicit_initializers() {
    let dir = std::env::temp_dir().join(format!("atto-init-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("main.atto");
    for body in ["val x: int;", "let x: int;", "val int x;", "let int x;"] {
        fs::write(&source, format!("pub fnc main()->int {{ {body} return 42; }}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object-strict").arg(&source).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{body}: {:?}", out);
        assert!(out.stdout.is_empty(), "{body}: emitted object");
        assert!(String::from_utf8_lossy(&out.stderr).contains("E_MISSING_INITIALIZER"), "{body}: {:?}", out);
    }
    fs::write(&source, "pub fnc main()->int { val x: int = 42; return x; }").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("emit-object-strict").arg(&source).output().unwrap();
    assert!(out.status.success(), "{:?}", out);
    assert!(out.stdout.starts_with(b"\x7fELF"));
    fs::remove_dir_all(dir).unwrap();
}
