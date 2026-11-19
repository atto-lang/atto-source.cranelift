#![cfg(unix)]
use std::fs::{self, OpenOptions};
use std::os::unix::fs::symlink;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn regular_file_symlink_is_rejected_before_guest_execution() {
    let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "orescranelift-source-admission-{}-{}",
        std::process::id(),
        nonce
    ));
    fs::create_dir(&directory).expect("create private fixture directory");
    let original = directory.join("real.ores");
    let alias = directory.join("alias.ores");
    use std::io::Write;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&original)
        .unwrap();
    file.write_all(b"pub fnc main(): int { return 42; }\n")
        .unwrap();
    drop(file);
    symlink(&original, &alias).unwrap();

    let bad = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&alias)
        .output()
        .expect("launch real CLI");
    assert!(!bad.status.success(), "symlinked input was admitted");
    assert!(bad.stdout.is_empty(), "rejected input printed guest result");
    assert!(
        !bad.stderr.is_empty(),
        "rejection did not emit a diagnostic"
    );

    let good = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&original)
        .output()
        .expect("launch real CLI");
    assert!(
        good.status.success(),
        "regular source unexpectedly denied: {}",
        String::from_utf8_lossy(&good.stderr)
    );
    assert_eq!(good.stdout, b"42\n");
    fs::remove_file(alias).unwrap();
    fs::remove_file(original).unwrap();
    fs::remove_dir(directory).unwrap();
}


#[test]
fn intermediate_directory_symlink_is_rejected_but_real_nested_paths_execute() {
    let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "atto-source-component-{}-{nonce}", std::process::id()
    ));
    let real = root.join("real");
    fs::create_dir_all(&real).expect("create isolated real directory");
    let source = real.join("entry.atto");
    fs::write(&source, "fnc main():int{return 42;}\n").unwrap();
    let alias = root.join("alias");
    symlink("real", &alias).expect("create symlinked intermediate directory");

    let admitted = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .args(["run"]).arg(&source).output().unwrap();
    assert_eq!(admitted.status.code(), Some(0), "real path: {}", String::from_utf8_lossy(&admitted.stderr));
    assert_eq!(admitted.stdout, b"42\n");

    for mode in ["run", "ir"] {
        let denied = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode).arg(alias.join("entry.atto")).output().unwrap();
        assert_eq!(denied.status.code(), Some(1), "{mode} followed intermediate symlink");
        assert!(denied.stdout.is_empty(), "{mode} emitted guest value or CLIF");
        assert!(!denied.stderr.is_empty());
    }
    fs::remove_file(alias).unwrap();
    fs::remove_file(source).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parent_traversal_is_rejected_before_opening_guest_source() {
    let source = std::path::Path::new("tests/fixtures/../fixtures/clients-scalar-v1/01-precedence.ores");
    for mode in ["run", "ir"] {
        let denied = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode).arg(source).output().unwrap();
        assert_eq!(denied.status.code(), Some(1), "{mode} admitted parent traversal");
        assert!(denied.stdout.is_empty());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("source parent traversal"));
    }
}
