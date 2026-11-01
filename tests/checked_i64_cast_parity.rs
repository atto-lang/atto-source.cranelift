//! Compiler-backed Cranelift i64 checked-identity-cast test candidates.
//! No host-language arithmetic is used as a substitute for guest execution.
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn native_checked_identity_casts_and_negative_admission() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "orescranelift-cast-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).expect("create isolated test directory");

    let positives = [
        ("literal-identity", "return (7 as int)+35;"),
        ("chained-identity", "return (7 as int as int)+35;"),
        ("precedence-multiply", "return ((2+5) as int)*6;"),
        ("nested-identity", "return 7*(6 as int);"),
        ("signed-boundary", "return (9223372036854775807 as int)-9223372036854775765;"),
        ("negative-times-negative", "return (-7 as int)*(-6 as int);"),
        ("inferred-const", "const value = 7 as int; return value*6;"),
        ("typed-readonly-local", "int value = 7 as int; return value*6;"),
    ];
    for (index, (name, body)) in positives.iter().enumerate() {
        let source = format!("pub fnc main():int{{{body}}}\n");
        let path = dir.join(format!("positive-{index:02}-{name}.ores"));
        std::fs::write(&path, source).expect("write guest source");
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .output()
            .expect("launch native Cranelift compiler/guest");
        assert!(
            output.status.success(),
            "native guest case {name} rejected: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"42\n", "unexpected native guest output: {name}");
    }

    let negatives = [
        ("bool-cast", "return 7 as bool;"),
        ("string-cast", "return 7 as string;"),
        ("nominal-cast", "return 7 as Object;"),
        ("chained-mismatch", "return 7 as int as bool;"),
        ("optional-cast", "return 7 as? int;"),
        ("bool-as-int", "return true as int;"),
        ("positive-overflow-literal", "return 9223372036854775808 as int;"),
        ("negative-overflow-literal", "return -9223372036854775809 as int;"),
        ("mutable-const", "const x=7 as int;x=8;return x;"),
        ("unknown-cast", "return 7 as Unknown;"),
    ];
    for (index, (name, body)) in negatives.iter().enumerate() {
        let path = dir.join(format!("negative-{index:02}-{name}.ores"));
        std::fs::write(&path, format!("pub fnc main():int{{{body}}}\n"))
            .expect("write negative guest source");
        for mode in ["run", "ir"] {
            let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .expect("launch actual compiler in negative mode");
            assert_eq!(
                result.status.code(),
                Some(1),
                "{name}: {mode} unexpectedly admitted unsupported source"
            );
            assert!(
                result.stdout.is_empty(),
                "{name}: {mode} produced guest output or CLIF on rejection"
            );
            assert!(!result.stderr.is_empty(), "{name}: missing compiler diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked"),
                "{name}: compiler panicked"
            );
        }
    }

    // Runtime overflow can be recognized only by the generated checked
    // machine code. The 'ir' command is allowed to emit a valid compiled
    // function for these inputs; it is NOT a runtime-negative proof.
    for (index, body) in [
        "return (9223372036854775807 as int)+1;",
        "int unused=(9223372036854775807 as int)+1;return 42;",
    ]
    .iter()
    .enumerate()
    {
        let path = dir.join(format!("runtime-overflow-{index:02}.ores"));
        std::fs::write(&path, format!("pub fnc main():int{{{body}}}\n"))
            .expect("write runtime overflow guest source");
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .output()
            .expect("launch native checked-overflow guest");
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty(), "overflow returned a guest value");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("overflow"),
            "expected compiler/JIT-owned checked overflow diagnostic"
        );
    }

    // Path must remain within our own test workspace.
    assert!(Path::new(&dir).is_absolute());
    std::fs::remove_dir_all(dir).expect("cleanup isolated test directory");
}
