//! Stage-0 native JIT must fail closed rather than silently accepting
//! source actors, tenant isolation, or capability-bearing host operations.
//! These tests execute the *actual* CLI on the Rust/Cranelift toolchain.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn reject_cli(mode: &str, source: &str) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-tenant-negative-{}-{}.ores",
        std::process::id(), id
    ));
    std::fs::write(&path, source).expect("write fixture");
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("run native compiler");
    std::fs::remove_file(&path).expect("remove fixture");
    assert!(!out.status.success(), "{mode} admitted unsupported tenant source: {source}");
    assert!(out.stdout.is_empty(), "{mode} emitted native guest result/IR on refusal: {source}");
    assert!(
        !out.stderr.is_empty(),
        "{mode} failed without an explicit diagnostic: {source}"
    );
}

#[test]
fn native_actor_tenant_ownership_and_scheduler_lowering_fail_closed() {
    // This backend currently supports only a strict signed-i64 scalar subset.
    // Treat all actor/tenant constructs as unsupported rather than silently
    // lowering to an unguarded host callback or claiming cross-backend parity.
    for source in [
        "define actor Worker as receive(int x) -> void :: end end",
        "define actor Worker as end",
        "fnc main():int{return spawn Worker();}",
        "fnc main():int{return await spawn Worker();}",
        "fnc main():int{return rt share 42;}",
        "fnc main():int{return rt mut share 42;}",
        "fnc main():int{return rt borrow mut 42;}",
        "fnc main():int{return handle_state(critical-write |s| -> {s.x = 1});}",
        "fnc main():int{return process.stdout;}",
        "fnc main():int{return stdio.out;}",
        "fnc main():int{return import native.host;}",
        "fnc main():int{return unsafe;}",
        "pub async fnc main():int{return 1;}",
        "fnc main():int{return 1;} define actor Extra as end",
    ] {
        for mode in ["run", "ir"] {
            reject_cli(mode, source);
        }
    }
}

#[test]
fn bounded_arguments_rejected_before_source_open_and_jit() {
    // This nonexistent file distinguishes the early argv gate from an
    // eventual source-load/JIT failure. Earlier code parsed source before
    // checking the bounded native entry ABI.
    let impossible = std::env::temp_dir().join(format!(
        "orescranelift-nonexistent-{}-{}.ores",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(!impossible.exists());
    for count in [5_usize, 8, 2048] {
        let args = vec!["1"; count];
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&impossible)
            .args(args)
            .output()
            .expect("invoke native compiler");
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("too many runtime arguments"),
            "unexpected early-admission diagnostic: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let ir = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("ir")
        .arg(&impossible)
        .arg("0")
        .output()
        .expect("invoke native compiler");
    assert!(!ir.status.success());
    assert!(ir.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&ir.stderr).contains("ir mode does not accept arguments")
    );
}
