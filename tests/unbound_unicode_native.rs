//! Ordinary native object compilation must distinguish unbound Unicode names
//! from unsupported scalar grammar and must not emit an object on rejection.
use std::{fs, process::Command};

#[test]
fn unbound_unicode_name_has_specific_diagnostic_and_no_object() {
    let dir = std::env::temp_dir().join(format!("atto-unbound-unicode-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("case.atto");
    for name in ["未定義", "𠮷", "ρ"] {
        fs::write(&source, format!("pub fnc main(): int {{ return {name}; }}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object").arg(&source).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        let diagnostic = String::from_utf8(out.stderr).unwrap();
        assert!(diagnostic.contains(&format!("E_UNBOUND_LOCAL: unbound identifier: {name}")), "{diagnostic}");
    }
    fs::write(&source, "pub fnc main(): int { val int ρ = 42; return ρ; }").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("emit-object").arg(&source).output().unwrap();
    assert!(out.status.success());
    assert!(!out.stdout.is_empty());
    fs::remove_dir_all(dir).unwrap();
}
