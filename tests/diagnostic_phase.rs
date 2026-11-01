//! Differential diagnostic admission: syntax/literal rejection must not
//! masquerade as a successfully lowered, checked native-JIT overflow.
//! These assertions only become guest evidence after executed exact-head CI.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let index = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-phase-{}-{index}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("fresh fixture directory");
        Self(path)
    }
    fn run(&self, id: &str, source_expr: &str) -> Output {
        let path = self.0.join(format!("{id}.ores"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("new source file");
        writeln!(file, "pub fnc main():int{{return {source_expr};}}")
            .expect("write source");
        file.flush().expect("flush source");
        drop(file);
        // Compilation failures must not be credited as runtime trap passes.
        Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(self.0.join(format!("{id}.ores")))
            .output()
            .expect("execute native compiler/JIT")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn unsupported_operators_reject_before_native_execution() {
    let fixture = Fixture::new();
    for (id, source) in [
        ("percent", "7 % 3"),
        ("nested_percent", "Integer.bitXor(7%3,1)"),
        ("caret", "7 ^ 3"),
        ("double_caret", "7 ^^ 3"),
        ("int_division", "7 / 2"),
    ] {
        let result = fixture.run(id, source);
        assert!(!result.status.success(), "{id} unexpectedly compiled");
        assert!(result.stdout.is_empty(), "{id} printed guest output");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.starts_with("orescranelift: "), "{id}: {stderr}");
        assert!(!stderr.contains("i64 arithmetic overflow"), "{id} misclassified");
    }
}
#[test]
fn invalid_literals_are_compile_rejections_not_overflow_traps() {
    let fixture = Fixture::new();
    for (id, source) in [
        ("high_literal", "9223372036854775808"),
        ("low_literal", "-9223372036854775809"),
        ("bad_separator", "1__0"),
        ("trailing_separator", "1_"),
    ] {
        let result = fixture.run(id, source);
        assert!(!result.status.success(), "{id} unexpectedly compiled");
        assert!(result.stdout.is_empty(), "{id} printed guest output");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.starts_with("orescranelift: "), "{id}: {stderr}");
        assert!(!stderr.contains("i64 arithmetic overflow"), "{id} misclassified");
    }
}
#[test]
fn checked_overflow_reports_runtime_path_and_no_wrapped_stdout() {
    let fixture = Fixture::new();
    for (id, source) in [
        ("max_plus_two", "9223372036854775807 + 2"),
        ("min_minus_one", "(-9223372036854775807 - 1) - 1"),
        ("positive_square", "3037000500 * 3037000500"),
        ("negative_square", "(-3037000500) * 3037000500"),
        ("negated_min", "-(-9223372036854775808)"),
    ] {
        let result = fixture.run(id, source);
        assert!(!result.status.success(), "{id} accepted checked overflow");
        assert!(result.stdout.is_empty(), "{id} emitted wrapped output");
        assert_eq!(
            String::from_utf8_lossy(&result.stderr),
            "orescranelift: i64 arithmetic overflow\n",
            "{id} did not reach the checked native-JIT runtime path"
        );
    }
}
