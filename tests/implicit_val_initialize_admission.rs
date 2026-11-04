//! Nominal fields and initializers are unsupported by stage-0 Cranelift.
//! Execute both compiler modes using an exclusive bounded .atto fixture.
#[path = "support/mod.rs"]
mod support;

fn invoke(mode: &str, source: &str) -> std::process::Output {
    support::run_source(mode, source.as_bytes(), &[])
}

#[test]
fn class_initialization_contract_is_denied_before_jit_or_clif_publication() {
    const PROBES: &[(&str, &str)] = &[
        ("implicit-val", "pub define class C as pub int id: 42; end"),
        ("explicit-val", "define class C as val int id: 42; end"),
        ("mutable-let", "define class C as let int id: 1; end"),
        ("final", "define class C as final int id: 1; end"),
        ("const", "define class C as const int id: 1; end"),
        (
            "arrow-double-colon",
            "define class C as final int id; initialize(int n) -> void :: self.id = n; end end",
        ),
        (
            "arrow-newline",
            "define class C as\n final int id;\n initialize(int n) -> void\n self.id = n;\n end\nend",
        ),
        (
            "colon-newline",
            "define class C as\n final int id;\n initialize(int n): void\n self.id = n;\n end\nend",
        ),
        (
            "colon-braces",
            "define class C as final int id; initialize(int n): void { self.id = n; } end",
        ),
        (
            "explicit-return",
            "define class C as initialize(): void { return; } end",
        ),
        ("no-void", "define class C as initialize(int n) { } end"),
        (
            "nonvoid",
            "define class C as initialize(int n) -> int { return n; } end",
        ),
        (
            "as-delimiter",
            "define class C as initialize(int n) as self.id = n; end end",
        ),
        (
            "double-fixed-write",
            "define class C as final int id; initialize(int n) -> void :: self.id=n; self.id=n; end end",
        ),
        (
            "missing-mutable",
            "define class C as let int id; initialize() -> void {} end",
        ),
        (
            "self-escape",
            "define class C as let int id; initialize(int n) -> void :: publish(self); self.id=n; end end",
        ),
        (
            "inherited-fixed",
            "define class Parent as val int id; end define class Child extends Parent as initialize() -> void {} end",
        ),
        ("nested-in-scalar", "fnc main():int{return initialize(42);}"),
        (
            "trailing-class",
            "fnc main():int{return 42;} define class C as val int id: 1; end",
        ),
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
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
}
