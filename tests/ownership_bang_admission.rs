//! Real Cranelift scalar CLI; unsupported ownership is denied before native JIT.
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

// CI may run tests concurrently in a shared temporary directory. Never
// overwrite another process's fixture (or follow an attacker-owned symlink).
struct SourceFixture {
    path: PathBuf,
}

impl SourceFixture {
    fn create(source: &str) -> Self {
        for _ in 0..64 {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ores-ownership-bang-{}-{sequence}.atto",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let fixture = Self { path };
                    file.write_all(source.as_bytes())
                        .expect("write exclusive ownership fixture");
                    file.sync_all().expect("sync exclusive ownership fixture");
                    return fixture;
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create exclusive ownership fixture: {error}"),
            }
        }
        panic!("could not allocate an exclusive ownership fixture after 64 attempts");
    }
}

impl Drop for SourceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn run_source(source: &str, mode: &str) -> Output {
    let fixture = SourceFixture::create(source);
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&fixture.path)
        .output()
        .expect("run Cranelift ownership fixture")
}

#[test]
fn rejects_ownership_prefixes_before_run_or_ir() {
    for expression in [
        "copy!42", "take!42", "borrow! value", "borrow!mut value",
        "borrow! mut value", "mut borrow! value", "mut borrow!value", "share!value",
        "borrow!\nmut value", "borrow!/* checked loan */mut value",
        "mut borrow!\nvalue",
    ] {
        for mode in ["run", "ir"] {
            let source = format!("fnc main():int{{return {expression};}}");
            let output = run_source(&source, mode);
            assert!(!output.status.success(), "{mode} accepted {expression}");
            assert!(output.stdout.is_empty(), "{mode} emitted output for {expression}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("OWNERSHIP_BANG_BACKEND_UNSUPPORTED"),
                "{mode} wrong diagnostic for {expression}: {stderr}");
        }
    }
}

#[test]
fn commented_ownership_does_not_poison_scalar_execution() {
    for mode in ["run", "ir"] {
        let output = run_source(
            "fnc main():int{/* borrow! mut item */return 42;}",
            mode,
        );
        assert!(output.status.success(), "{mode}: {}",
            String::from_utf8_lossy(&output.stderr));
        assert!(!output.stdout.is_empty(), "{mode} produced no output");
    }
}

#[test]
fn exclusive_fixture_is_removed_on_scope_exit() {
    let path = {
        let fixture = SourceFixture::create("fnc main():int{return 42;}");
        assert!(fixture.path.is_file());
        assert_eq!(
            std::fs::read_to_string(&fixture.path).expect("read exclusive fixture"),
            "fnc main():int{return 42;}"
        );
        fixture.path.clone()
    };
    assert!(!path.exists(), "exclusive fixture leaked after use");
}
