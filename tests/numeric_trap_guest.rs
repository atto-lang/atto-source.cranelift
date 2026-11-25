//! Compiled guest evidence for the narrow numeric trap entry slice.
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("atto-numeric-trap-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn execute(&self, source: &str, mode: &str, args: &[&str]) -> std::process::Output {
        let path = self.0.join("guest.atto");
        fs::write(&path, source).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode).arg(path).args(args).output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
#[test]
fn native_numeric_trap_returns_distinct_tags_and_error_identities() {
    let f = Fixture::new();
    for spelling in ["trap", "trap!"] {
        for (expr, left, right, expected) in [
            ("a / b", "-7", "3", "Ok(-2)"),
            ("a / b", "7", "0", "Err(65538)"),
            ("a / b", "-9223372036854775808", "-1", "Err(65537)"),
            ("a + b", "9223372036854775807", "1", "Err(65537)"),
            ("a - b", "-9223372036854775808", "1", "Err(65537)"),
            ("a * b", "3037000500", "3037000500", "Err(65537)"),
            ("-a", "-9223372036854775808", "0", "Err(65537)"),
            ("Integer.rem(a,b)", "-7", "3", "Ok(-1)"),
            ("Integer.rem(a,b)", "7", "0", "Err(65538)"),
            ("Integer.mod(a,b)", "-7", "-3", "Ok(2)"),
            ("Integer.mod(a,b)", "7", "0", "Err(65538)"),
            ("(a + 1) / b", "9223372036854775807", "0", "Err(65537)"),
            ("(a / b) + 1", "9223372036854775807", "0", "Err(65538)"),
            ("a + b", "40", "2", "Ok(42)"),
        ] {
            let source = format!("pub fnc main(a:int,b:int):Result<int, Error>{{return {spelling} ({expr});}}");
            let output = f.execute(&source, "run", &[left, right]);
            assert_eq!(output.status.code(), Some(0), "{source}: {:?}", output.stderr);
            assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{expected}\n"));
            assert!(output.stderr.is_empty(), "{source}");
        }
    }
}
#[test]
fn result_unsupported_forms_fail_before_native_ir() {
    let f = Fixture::new();
    for source in [
        "fnc main():Result<int,int>{return trap! (7);}",
        "fnc main():Result<int,Error>{return 7;}",
        "fnc main():Result<int,Error>{return Ok(7);}",
        "fnc main():int{return trap! (7);}",
        "fnc main():Result<int,Error>{return trap! (Integer.pow(2,-1));}",
        "fnc main():Result<int,Error>{return trap! (trap! (7));}",
        "fnc main():Result<int,Error>{return trap! (7) + 1;}",
        "fnc main():Result<int,Error>{return trap ! (7);}",
        "fnc main():Result<int,Error>{return trap!(7);}",
        "fnc main():Result<int,Error>{return trap!7;}",
        "fnc main():Result<int,Error>{return trap!/*comment*/ (7);}",
        "fnc main():Option<int>{return Some(7);}",
    ] {
        for mode in ["check", "ir", "run"] {
            let output = f.execute(source, mode, &[]);
            assert_eq!(output.status.code(), Some(1), "{mode}: {source}");
            assert!(output.stdout.is_empty(), "{mode}: {source}");
            assert!(!output.stderr.is_empty(), "{mode}: {source}");
        }
    }
}
#[test]
fn numeric_result_entry_never_enters_scalar_isolate_path() {
    let f = Fixture::new();
    let source = "fnc main(a:int,b:int):Result<int,Error>{return trap! (a+b);}";
    let output = f.execute(source, "isolate", &["1","2"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn bang_keyword_suffix_requires_whitespace() {
    let f = Fixture::new();
    for verb in ["trap", "throw", "raise", "panic", "copy", "take", "borrow", "share", "pin", "drop", "pipe", "rpipe", "stop", "expand", "copy!"] {
        for tail in ["(7)", "value"] {
            let source = format!("fnc main():int{{return {verb}!{tail};}}");
            let output = f.execute(&source, "check", &[]);
            assert_eq!(output.status.code(), Some(1), "{source}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("E_BANG_KEYWORD_WHITESPACE"), "{source}: {:?}", output.stderr);
        }
    }
}
