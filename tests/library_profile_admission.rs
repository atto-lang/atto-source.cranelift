//! Explicit Core/Alloc/Std rejection in the narrow stage-0 Cranelift frontend.
//! A rejected source must never emit native JIT output or IR bytes.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn invoke(source: &str, mode: &str) -> std::process::Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path=std::env::temp_dir().join(format!(
        "atto-cranelift-library-{}-{id}.atto",std::process::id()));
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).expect("exclusive candidate");
    file.write_all(source.as_bytes()).expect("write candidate");
    drop(file);
    let output=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode).arg(&path).output().expect("run compiler");
    std::fs::remove_file(path).expect("cleanup");
    output
}

#[test]
fn canonical_and_legacy_library_imports_are_rejected_before_ir() {
    for spec in [
        "atto.core/numeric", "atto.core/numeric.atto",
        "atto.alloc/byte_region", "atto.std/fs", "atto.runtime.actor",
        "ores.core/numeric", "ores.std/core",
        "./atto.core/numeric.atto", "../atto.std/fs.atto"
    ] {
        let source=format!(
            "import module library from \"{spec}\";\nfnc main(): int {{ return 42; }}"
        );
        for mode in ["check", "run", "ir"] {
            let output=invoke(&source,mode);
            assert!(!output.status.success(), "{mode} accepted {spec}");
            assert!(output.stdout.is_empty(), "{mode} emitted an artifact for {spec}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("E_LIBRARY_PROFILE_UNSUPPORTED"),
                "{mode}: wrong diagnostic for {spec}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn comments_with_import_words_do_not_block_pure_compiler_execution() {
    let source="// import module fs from \"atto.std/fs\";\n\
        fnc main(): int { return 42; }\n";
    let output=invoke(source,"run");
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout,b"42\n");
}
