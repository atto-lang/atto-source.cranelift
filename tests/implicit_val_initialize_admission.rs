//! Stage-0 cannot compile nominal class fields or initializers. Keep every
//! proposed backend-neutral construction form fail-closed in both JIT and IR
//! modes until the actual checked class layout/ownership backend exists.
//! These are real CLI admission tests; rejection is NOT feature parity.
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct SourceFile(PathBuf);
impl SourceFile {
    fn new(source: &str) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let file = std::env::temp_dir().join(format!(
            "orescranelift-initialize-v1-{}-{id}.ores",
            std::process::id()
        ));
        std::fs::write(&file, source).expect("create isolated fixture");
        Self(file)
    }
}
impl Drop for SourceFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn invoke(mode: &str, source: &str) -> Output {
    let file = SourceFile::new(source);
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&file.0)
        .output()
        .expect("run actual stage-0 CLI")
}

#[test]
fn class_initialization_contract_is_denied_before_jit_or_clif_publication() {
    const PROBES: &[(&str, &str)] = &[
        ("implicit-val", "pub define class C as pub int id: 42; end"),
        ("explicit-val", "define class C as val int id: 42; end"),
        ("mutable-let", "define class C as let int id: 1; end"),
        ("final", "define class C as final int id: 1; end"),
        ("const", "define class C as const int id: 1; end"),
        ("arrow-double-colon", "define class C as final int id; initialize(int n) -> void :: self.id = n; end end"),
        ("arrow-newline", "define class C as\n final int id;\n initialize(int n) -> void\n self.id = n;\n end\nend"),
        ("colon-newline", "define class C as\n final int id;\n initialize(int n): void\n self.id = n;\n end\nend"),
        ("colon-braces", "define class C as final int id; initialize(int n): void { self.id = n; } end"),
        ("explicit-return", "define class C as initialize(): void { return; } end"),
        ("no-void", "define class C as initialize(int n) { } end"),
        ("nonvoid", "define class C as initialize(int n) -> int { return n; } end"),
        ("as-delimiter", "define class C as initialize(int n) as self.id = n; end end"),
        ("double-fixed-write", "define class C as final int id; initialize(int n) -> void :: self.id=n; self.id=n; end end"),
        ("missing-mutable", "define class C as let int id; initialize() -> void {} end"),
        ("self-escape", "define class C as let int id; initialize(int n) -> void :: publish(self); self.id=n; end end"),
        ("inherited-fixed", "define class Parent as val int id; end define class Child extends Parent as initialize() -> void {} end"),
        ("nested-in-scalar", "fnc main():int{return initialize(42);}"),
        ("trailing-class", "fnc main():int{return 42;} define class C as val int id: 1; end"),
    ];
    for (id, source) in PROBES {
        for mode in ["run", "ir"] {
            let output = invoke(mode, source);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{id}: {mode} admitted unsupported class construction"
            );
            assert!(
                output.stdout.is_empty(),
                "{id}: {mode} published guest result or partial Cranelift IR"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("orescranelift:"),
                "{id}: {mode} emitted no CLI diagnostic: {stderr}"
            );
            assert!(
                !stderr.contains("panicked at") && !stderr.contains("stack backtrace"),
                "{id}: {mode} failed by compiler panic: {stderr}"
            );
        }
    }
}

#[test]
fn initializer_text_inside_comments_does_not_block_a_scalar_guest() {
    let source = "fnc main():int{return 40+2;} // initialize(int n) -> void :: val int id;\n";
    let output = invoke("run", source);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"42\n");
}
