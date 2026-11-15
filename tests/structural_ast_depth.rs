use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn invoke(mode: &str, extension: &str, source: &str) -> Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-ast-depth-{}-{sequence}.{extension}",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("create isolated scalar source");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("invoke real Cranelift CLI");
    std::fs::remove_file(&path).expect("remove scalar source");
    output
}

fn addition_chain(terms: usize) -> String {
    let expr = vec!["1"; terms].join("+");
    format!("fnc main():int{{return {expr};}}")
}

#[test]
fn bounded_associative_chains_admit_native_guest_and_verified_ir() {
    let source = addition_chain(32);
    for extension in ["atto", "ores"] {
        let run = invoke("run", extension, &source);
        assert_eq!(run.status.code(), Some(0), "{}", String::from_utf8_lossy(&run.stderr));
        assert_eq!(run.stdout, b"32\n");
        assert!(run.stderr.is_empty());

        let ir = invoke("ir", extension, &source);
        assert_eq!(ir.status.code(), Some(0), "{}", String::from_utf8_lossy(&ir.stderr));
        assert!(!ir.stdout.is_empty(), "successful IR must not be empty");
        assert!(ir.stderr.is_empty());
    }
}

#[test]
fn long_left_spines_reject_before_ir_or_jit_without_stdout() {
    // This chain is shallow in recursive-descent call depth, but the
    // constructed left-associated expression tree exceeds MAX_DEPTH=128.
    let source = addition_chain(130);
    for extension in ["atto", "ores"] {
        for mode in ["run", "ir"] {
            let output = invoke(mode, extension, &source);
            assert_eq!(output.status.code(), Some(1), "{mode} {extension}: {}", String::from_utf8_lossy(&output.stderr));
            assert!(output.stdout.is_empty(), "{mode} {extension} emitted output");
            assert_eq!(
                String::from_utf8_lossy(&output.stderr),
                "orescranelift: expression depth budget exceeded\n",
                "{mode} {extension} must fail through frontend admission"
            );
        }
    }
}
