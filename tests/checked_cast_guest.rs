//! Exact Cranelift source -> CLIF -> native guest admission for checked i64 casts.
//! Scope: checked identity casts only. Neither host arithmetic nor full cast parity.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn guest(source: &str, mode: &str) -> std::process::Output {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be monotonic after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "orescranelift-checked-cast-{}-{timestamp}-{}.ores",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).expect("cannot write isolated guest source");
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("cannot start Cranelift compiler");
    std::fs::remove_file(path).expect("cannot clean up guest source");
    result
}

#[test]
fn native_checked_identity_casts_return_signed_i64() {
    for (source, expected) in [
        ("pub fnc main():int{return (7 as int)+35;}", "42\n"),
        ("pub fnc main():int{return (7 as int as int)+35;}", "42\n"),
        ("pub fnc main():int{return ((2+5) as int)*6;}", "42\n"),
        ("pub fnc main():int{return (7*6) as int;}", "42\n"),
        ("pub fnc main():int{return (-(-42)) as int;}", "42\n"),
        ("pub fnc main():int{const x=7 as int;return x*6;}", "42\n"),
        ("pub fnc main():int{const x: int=7 as int;return x*6;}", "42\n"),
        ("pub fnc main():int{int x=7;return (x as int)*6;}", "42\n"),
        (
            "pub fnc main():int{return -9223372036854775808 as int;}",
            "-9223372036854775808\n",
        ),
    ] {
        let output = guest(source, "run");
        assert!(
            output.status.success(),
            "guest rejected: {source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes(), "wrong guest value: {source}");
    }
}

#[test]
fn unsupported_checked_casts_emit_no_guest_or_clif() {
    for (source, diagnostic) in [
        ("pub fnc main():int{return 7 as bool;}", "unsupported checked cast target"),
        ("pub fnc main():int{return 7 as string;}", "unsupported checked cast target"),
        ("pub fnc main():int{return 7 as Object;}", "unsupported checked cast target"),
        ("pub fnc main():int{return 7 as int as bool;}", "unsupported checked cast target"),
        ("pub fnc main():int{return 7 as? int;}", "invalid character"),
        ("pub fnc main():int{return true as int;}", "unbound or unsupported identifier"),
        (
            "pub fnc main():int{return 9223372036854775808 as int;}",
            "i64 literal out of range",
        ),
        (
            "pub fnc main():int{return 7 as int + 9223372036854775808;}",
            "i64 literal out of range",
        ),
        ("pub fnc main():int{return 7 % 3 as int;}", "operator '%' is removed"),
    ] {
        for mode in ["run", "ir"] {
            let output = guest(source, mode);
            assert_eq!(output.status.code(), Some(1), "must reject: {source}");
            assert!(output.stdout.is_empty(), "invalid source emitted guest/CLIF: {source}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(diagnostic), "wrong diagnostic for {source}: {stderr}");
            assert!(!stderr.contains("panicked"), "compiler crashed: {stderr}");
        }
    }
}

#[test]
fn exact_byte_five_backend_source_guests() {
    // These strings are compiled directly from the identical .ores files in
    // Java, LLVM, Wasm, .NET and Cranelift. No backend-specific source rewrite.
    const POSITIVE: &str =
        include_str!("../contracts/v1/portable-checked-cast-positive-v2.ores");
    const NEGATIVE: &str =
        include_str!("../contracts/v1/portable-checked-cast-negative-v2.ores");
    let output = guest(POSITIVE, "run");
    assert!(
        output.status.success(),
        "source-identical positive guest failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.as_slice(), b"42\n");
    for mode in ["run", "ir"] {
        let rejected = guest(NEGATIVE, mode);
        assert_eq!(rejected.status.code(), Some(1), "invalid cast admitted");
        assert!(rejected.stdout.is_empty(), "invalid cast emitted CLIF/guest value");
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("unsupported checked cast target"),
            "negative source was not rejected by checked-cast admission"
        );
    }
}
