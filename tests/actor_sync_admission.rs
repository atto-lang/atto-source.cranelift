//! Actor Sync v1 conformance *negative admission only*.
//! This stage-0 Cranelift backend compiles scalar native JIT programs, not
//! source actors, durable guest APIs or actor-owned heaps. These checks may
//! NEVER be recorded as a positive AS01-AS24 native actor parity receipt.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn invoke(mode: &str, source: &str) -> std::process::Output {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-sync-negative-{}-{nonce}.ores",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write negative fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("invoke Cranelift executable");
    std::fs::remove_file(path).expect("remove fixture");
    output
}

#[test]
fn actor_sync_guest_source_is_not_silently_accepted_as_scalar_jit() {
    let cases = [
        (
            "isolated-actor",
            "define isolated actor Cart as state int count; end",
        ),
        (
            "actor-plus-main",
            "define isolated actor Cart as end fnc main():int{return 42;}",
        ),
        ("actor-state", "define actor Cart as state int count; end"),
        ("virtual-ref", "fnc main():int{return ActorRef<Cart>(0);}"),
        ("spawn", "fnc main():int{return spawn Cart();}"),
        ("watch", "fnc main():int{return watch(cart, 0);}"),
        ("snapshot", "fnc main():int{return snapshot(cart);}"),
        ("suspends", "suspends fnc main():int{return 42;}"),
        ("await", "fnc main():int{return await pending;}"),
        ("critical", "critical fnc main():int{return 42;}"),
        ("critical-block", "fnc main():int{critical {return 42;}}"),
        ("rt-borrow", "fnc main():int{return rt borrow 42;}"),
    ];
    for (label, source) in cases {
        for mode in ["run", "ir"] {
            let output = invoke(mode, source);
            assert!(
                !output.status.success(),
                "{mode} unexpectedly admitted actor-sync {label} as scalar program"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} generated native value / IR for unsupported {label}"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!stderr.is_empty(), "missing diagnostic for {label}");
            assert!(
                !stderr.to_lowercase().contains("panicked"),
                "{mode} panicked for {label}: {stderr}"
            );
        }
    }
}

#[test]
fn actor_sync_words_in_comments_do_not_break_supported_scalar_guest() {
    let output = invoke(
        "run",
        "fnc main():int{return 42;} // actor sync watch critical\n",
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
}

#[test]
fn grouped_spawn_rejects_before_native_value_or_ir() {
    for expr in [
        "spawn Worker() with {}",
        "spawn Worker() with { schedule { max_parallelism = 2; } }",
        "spawn Worker() with { resources { heap_size = 1024; } }",
        "spawn Worker() with { init { seed = 42; } }",
        "spawn Worker() /* trivia */ with { init {} }",
    ] {
        let source = format!("fnc main():int{{return {expr};}}");
        for mode in ["run", "ir"] {
            let output = invoke(mode, &source);
            assert!(!output.status.success(), "{mode}: {expr}");
            assert!(output.stdout.is_empty(), "{mode}: emitted artifact for {expr}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("actor and supervisor capabilities are unsupported"), "{mode}: {expr}: {stderr}");
        }
    }
    let output = invoke("run", "fnc main():int{return 42;} // spawn Worker() with {}\n");
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(output.stdout, b"42\n");
}
