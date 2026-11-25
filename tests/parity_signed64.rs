//! This candidate suite uses source-identical bytes from parity-testing PR #19.
//! Authored executable CLI/JIT regression: no GitHub/CI guest receipt implied.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_source(source: &str, should_succeed: bool, expected: &str) {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-cranelift-parity-{}-{n}.ores",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("create isolated exact-byte guest fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("spawn real native Cranelift CLI");
    std::fs::remove_file(path).expect("remove isolated guest fixture");
    assert_eq!(
        output.status.success(),
        should_succeed,
        "CLI status / stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if should_succeed {
        assert_eq!(
            output.stdout,
            expected.as_bytes(),
            "guest result bytes differ"
        );
        assert!(
            output.stderr.is_empty(),
            "successful source emitted diagnostics"
        );
    } else {
        assert!(
            output.stdout.is_empty(),
            "negative guest must not emit stdout"
        );
        assert!(
            !output.stderr.is_empty(),
            "negative guest must carry a diagnostic"
        );
    }
}

#[test]
fn ten_shared_signed_i64_candidates_execute_actual_cranelift_jit() {
    // Same Oreslang source bytes as parity-testing/PR#19 programs 40..49.
    let examples: &[(&str, &str)] = &[
        (
            "40-wide-i64-difference",
            "// 40: wide-i64-difference\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return 9007199254740993 - 9007199254740951; }\n",
        ),
        (
            "41-min-i64-successor",
            "// 41: min-i64-successor\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return (-9223372036854775807 - 1) + 9223372036854775807 + 43; }\n",
        ),
        (
            "42-max-i64-predecessor",
            "// 42: max-i64-predecessor\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return 9223372036854775807 - (9223372036854775807 - 42); }\n",
        ),
        (
            "43-wide-checked-product",
            "// 43: wide-checked-product\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return 3037000499 * 3037000499 - 9223372030926248959; }\n",
        ),
        (
            "44-double-negative-product",
            "// 44: double-negative-product\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return (1 - 43) * -1; }\n",
        ),
        (
            "45-grouped-negation",
            "// 45: grouped-negation\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return -(-42); }\n",
        ),
        (
            "46-named-bitxor",
            "// 46: named-bitxor\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return Integer.bitXor(40, 2); }\n",
        ),
        (
            "47-mantissa-precision-product",
            "// 47: mantissa-precision-product\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return (9007199254740993 - 9007199254740992) * 42; }\n",
        ),
        (
            "48-max-i64-delta",
            "// 48: max-i64-delta\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return 9223372036854775807 - 9223372036854775765; }\n",
        ),
        (
            "49-signed-boundary-product",
            "// 49: signed-boundary-product\n// Candidate scalar exactness fixture; no compiler-backed execution claimed.\npub fnc main(): int { return (9223372036854775807 - 9223372036854775800) * 6; }\n",
        ),
    ];
    assert_eq!(examples.len(), 10);
    for (name, source) in examples {
        assert!(name.len() > 3);
        run_source(source, true, "42\n");
    }
}

#[test]
fn unimplemented_type_adt_actor_async_features_remain_fail_closed() {
    for source in [
        "pub fnc main(): int { const x = 7; return x; }",
        "pub fnc main(): int { return 7 as bool; }",
        "define enum Status as Ready, Failed end pub fnc main(): int { return 42; }",
        "define isolated actor Counter as state { value: int = 0; } end pub fnc main(): int { return 42; }",
        "pub async fnc main(): int { return 42; }",
        "pub fnc main(): int { return Future<int>(42); }",
    ] {
        run_source(source, false, "");
    }
}

#[test]
fn identity_int_cast_is_supported_without_type_conversion() {
    run_source("pub fnc main(): int { return 7 as int; }", true, "7\n");
}
