//! Actual Cranelift CLI gates for reserved cpu/gpu/quantum execution domains.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_source(source: &str, mode: &str, args: &[&str]) -> std::process::Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-execution-domain-{}-{sequence}.ores",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    output
}

fn rejected_with_diagnostic(source: &str, diagnostic: &str) {
    for mode in ["run", "ir"] {
        let output = run_source(source, mode, &[]);
        assert!(!output.status.success(), "{mode} accepted {source}");
        assert!(
            output.stdout.is_empty(),
            "{mode} emitted output for {source}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(diagnostic),
            "{mode} wrong diagnostic for {source}: {stderr}"
        );
    }
}

fn rejected_without_output(source: &str) {
    rejected_with_diagnostic(source, "EXECUTION_DOMAIN_UNSUPPORTED");
}

#[test]
fn domain_words_are_reserved_in_every_position() {
    for word in ["cpu", "gpu", "quantum"] {
        rejected_without_output(&format!("fnc main(int {word}): int {{ return {word}; }}\n"));
        rejected_without_output(&format!(
            "fnc main({word}: int): int {{ return {word}; }}\n"
        ));
        rejected_without_output(&format!("fnc main(): int {{ return {word} {{ 1 }}; }}\n"));
        rejected_without_output(&format!("{word} fnc main(): int {{ return 1; }}\n"));
        rejected_without_output(&format!(
            "fnc main(): int {{ return {word} task {{ 1 }}; }}\n"
        ));
        rejected_without_output(&format!("fnc {word}(): int {{ return 1; }}\n"));
    }
}

#[test]
fn task_and_nb_blocks_are_rejected_as_execution_tasks() {
    rejected_with_diagnostic("fnc main(): int { return task { 1 }; }\n", "E_TASK_UNSUPPORTED");
    rejected_with_diagnostic("fnc main(): int { return task // block\n { 1 }; }\n", "E_TASK_UNSUPPORTED");
    rejected_without_output("fnc main(): int { return nb { 1 }; }\n");
}

#[test]
fn comments_and_longer_identifiers_stay_ordinary() {
    let source = "// gpu { cpu { quantum } } task { }\n\
                  fnc main(int task, int gpu_count): int {\n\
                  return task + gpu_count;\n\
                  }\n";
    let output = run_source(source, "run", &["40", "2"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}
