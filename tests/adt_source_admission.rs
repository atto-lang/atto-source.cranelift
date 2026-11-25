use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ADT: AtomicU64 = AtomicU64::new(0);

// Byte-identical Oreslang sources from platform-conformance #270.
// A generic unsupported error is an admission reservation, NOT semantic parity.
const ADT_CASES: &[(&str, &[u8], bool)] = &[
    (
        "nullary",
        include_bytes!("fixtures/adt-v1/nullary.ores"),
        true,
    ),
    (
        "payload",
        include_bytes!("fixtures/adt-v1/payload.ores"),
        true,
    ),
    ("owner", include_bytes!("fixtures/adt-v1/owner.ores"), true),
    (
        "max-tag",
        include_bytes!("fixtures/adt-v1/max-tag.ores"),
        true,
    ),
    (
        "guarded-not-exhaustive",
        include_bytes!("fixtures/adt-v1/guarded-not-exhaustive.ores"),
        true,
    ),
    (
        "direct-unboxed-recursion",
        include_bytes!("fixtures/adt-v1/direct-unboxed-recursion.ores"),
        true,
    ),
    (
        "option-result-partitions",
        include_bytes!("fixtures/adt-v1/option-result-partitions.ores"),
        false,
    ),
    (
        "option-partial-int",
        include_bytes!("fixtures/adt-v1/option-partial-int.ores"),
        false,
    ),
];

fn invoke(mode: &str, source: &[u8]) -> std::process::Output {
    let sequence = NEXT_ADT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-adt-{}-{sequence}.ores",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write canonical ADT fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("invoke native compiler");
    std::fs::remove_file(path).expect("remove temporary fixture");
    output
}

#[test]
fn canonical_adt_guest_sources_refuse_before_cranelift_ir_or_jit() {
    for &(name, source, declares_enum) in ADT_CASES {
        for mode in ["run", "ir"] {
            let output = invoke(mode, source);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{name}/{mode}: unsupported ADT source must fail admission"
            );
            assert!(
                output.stdout.is_empty(),
                "{name}/{mode}: no machine result or CLIF must be published"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.starts_with("orescranelift: "),
                "{name}/{mode}: missing compiler diagnostic: {stderr}"
            );
            assert!(
                !stderr.contains("panicked"),
                "{name}/{mode}: compiler panic is not semantic rejection"
            );
            if declares_enum {
                assert!(
                    stderr.contains("DECL_KIND_UNSUPPORTED: enum"),
                    "{name}/{mode}: wrong unsupported capability gate: {stderr}"
                );
            }
        }
    }
}

#[test]
fn scalar_control_still_executes_and_emits_clif() {
    let source = b"fnc main():int{return 42;}\n";
    let run = invoke("run", source);
    assert!(
        run.status.success(),
        "scalar run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"42\n");
    let ir = invoke("ir", source);
    assert!(
        ir.status.success(),
        "scalar IR failed: {}",
        String::from_utf8_lossy(&ir.stderr)
    );
    let clif = String::from_utf8_lossy(&ir.stdout);
    assert!(
        clif.contains("function") && clif.contains("42"),
        "expected real Cranelift function body, not a skipped check"
    );
}

#[test]
fn type_only_patterns_refuse_before_ir_or_jit() {
    for (source, diagnostic) in [
        (
            b"define pattern P as end".as_slice(),
            "native type-only ADT pattern lowering unsupported",
        ),
        (
            b"match value over end".as_slice(),
            "MATCH_DEFAULT_BACKEND_UNSUPPORTED",
        ),
    ] {
        for mode in ["run", "ir"] {
            let output = invoke(mode, source);
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(diagnostic), "{mode}: {stderr}");
        }
    }
}
