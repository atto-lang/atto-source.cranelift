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
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write fixture");
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("run native compiler");
    std::fs::remove_file(&path).expect("remove fixture");
    assert!(
        !out.status.success(),
        "{mode} admitted unsupported tenant source: {source}"
    );
    assert!(
        out.stdout.is_empty(),
        "{mode} emitted native guest result/IR on refusal: {source}"
    );
    assert!(
        !out.stderr.is_empty(),
        "{mode} failed without an explicit diagnostic: {source}"
    );
    let diagnostic = String::from_utf8_lossy(&out.stderr);
    assert!(
        diagnostic.starts_with("orescranelift: "),
        "{mode} did not return a compiler-owned error: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("panicked at") && !diagnostic.contains("No such file"),
        "{mode} failed before source admission or panicked: {diagnostic}"
    );
}

#[test]
fn scalar_positive_controls_prove_native_run_and_ir_still_work() {
    // Negatives alone are not semantic evidence: a compiler that rejects
    // everything would appear safe. Exercise one real native guest and IR.
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-scalar-positive-{}-{}.ores",
        std::process::id(),
        id
    ));
    std::fs::write(&path, "fnc main():int{return (6*7);}\n").expect("write positive fixture");
    for mode in ["run", "ir"] {
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&path)
            .output()
            .expect("execute native compiler positive control");
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty(), "{mode}: unexpected diagnostic");
        if mode == "run" {
            assert_eq!(out.stdout, b"42\n", "native guest scalar result");
        } else {
            assert!(
                !out.stdout.is_empty(),
                "successful native IR must not be empty"
            );
        }
    }
    std::fs::remove_file(&path).expect("remove positive fixture");
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
fn actor_private_heap_and_loan_syntax_never_lowers_as_scalar_jit() {
    // Acceptance remains explicitly UNSUPPORTED on this stage-0 Cranelift
    // compiler. Do not mistake rejection for successful actor parity.
    // Each program is exercised through the native run and IR CLI paths.
    let programs = [
        r#"define isolated actor Counter as
             state { count: int = 0; }
             pub on_message(int delta): int using state.write {
                 return self.handle_state(critical-write |s| -> {
                     s.count = s.count + delta;
                     return s.count;
                 });
             }
           end
           pub fnc main(): int { return 0; }"#,
        r#"define actor Worker as
             state { total: int = 0; }
             pub on_message(int n): int using state.write {
                 return self.handle_state(critical-write |s| -> {
                     s.total = s.total + n;
                     return s.total;
                 });
             }
           end
           pub fnc main(): int { return 0; }"#,
        "fnc main():int{return rt take 7;}",
        "fnc main():int{return rt copy 7;}",
        "fnc main():int{return rt borrow 7;}",
        "fnc main():int{return rt share 7;}",
        "fnc main():int{return rt mut borrow 7;}",
        "fnc main():int{return rt mut share 7;}",
        "fnc main():int{return spawn Worker();}",
        "fnc main():int{return await readch ch;}",
        "fnc main():int{return self.handle_state(critical-read |s| -> {return s.count;});}",
        "fnc main():int{return self.handle_state(critical-write |s| -> {s.count = 7;});}",
    ];
    for program in programs {
        for mode in ["run", "ir"] {
            reject_cli(mode, program);
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
        String::from_utf8_lossy(&ir.stderr).contains("ir/check mode does not accept runtime arguments")
    );
}
