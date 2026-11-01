//! Actual Cranelift frontend CLI gates, not a model of future nominal support.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_source(source: &str, mode: &str) -> std::process::Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-declaration-kind-{}-{sequence}.ores",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    output
}

#[test]
fn rejects_every_unsupported_nominal_declaration_without_output() {
    let declarations = [
        "define actor Worker as\nend\n",
        "define supervisor Guardian as\nend\n",
        "define grain User as\nend\n",
        "define state container Inventory as\nend\n",
        "define model Network as\nend\n",
        "define enum Mode as\nend\n",
        "define class Counter as\nend\n",
        "define struct Point as\nend\n",
        "define record Pair as\nend\n",
        "define interface Message as\nend\n",
        "define trait Sendable as\nend\n",
        "define protocol Wire as\nend\n",
        "define service Router as\nend\n",
        "define actor A extends B implements I as\nend\n",
        "define supervisor S extends A as\nend\n",
        "define state container S implements I as\nend\n",
        "define grain G extends S as\nend\n",
    ];
    for source in declarations {
        for mode in ["run", "ir"] {
            let output = run_source(source, mode);
            assert!(!output.status.success(), "{mode} accepted {source}");
            assert!(output.stdout.is_empty(), "{mode} emitted output for {source}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("DECL_KIND_UNSUPPORTED"),
                "{mode} wrong diagnostic for {source}: {stderr}"
            );
        }
    }
}

#[test]
fn scalar_main_remains_executable_after_declaration_guard() {
    for mode in ["run", "ir"] {
        let output = run_source("define fnc main():int{return 7;}\n", mode);
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
    }
}
