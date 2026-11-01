//! Actor and differentiable pipeline syntax is outside Cranelift stage-0.
//! Execute the actual compiler in BOTH modes. A diagnostic-only denial is a
//! successful safety test, never actor or ML implementation evidence.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn reject_both_modes(label: &str, source: &str) {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-actor-denial-{}-{}-{nonce}.ores",
        std::process::id(),
        label
    ));
    std::fs::write(&path, source).expect("write test fixture");
    for mode in ["run", "ir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&path)
            .output()
            .expect("invoke native compiler");
        assert!(!output.status.success(), "{label}/{mode} was silently admitted");
        assert!(output.stdout.is_empty(), "{label}/{mode} emitted output or native IR");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.trim().is_empty(), "{label}/{mode} must diagnose rejection");
        assert!(!stderr.contains("thread 'main' panicked"), "{label}/{mode} crashed: {stderr}");
    }
    std::fs::remove_file(path).expect("remove test fixture");
}

#[test]
fn actor_class_hierarchy_and_protocol_fail_closed() {
    for (label, src) in [
        ("basic", "define actor A1 as end\nfnc main():int{return 42;}"),
        ("shared", "define shared actor A1 as end\nfnc main():int{return 42;}"),
        ("inherited", "define actor Parent as end\ndefine actor Child extends Parent as end"),
        ("class-as-actor", "define class Child extends Actor as end"),
        ("ordinary-class-parent", "define class Plain as end\ndefine actor Bad extends Plain as end"),
        ("two-parents", "define actor A as end\ndefine actor B as end\ndefine actor C extends A, B as end"),
        ("interface", "define interface Reader as read(): int; end\ndefine actor A implements Reader as end"),
        ("trait", "define trait T as step(): void; end\ndefine actor A implements T as end"),
        ("supervisor", "define supervisor S1 as end\ndefine actor A1 extends S1 as end"),
        ("isolated", "define isoactor A1 as end"),
        ("untrusted", "define untrusted actor A1 as end"),
        ("runtime-method", "define actor A1 as pub tick(): void { return; } end"),
        ("on-handler", "define actor A1 as on #tick :: return; end end"),
        ("sealed-state", "define actor A1 as state { int count = 0; } end"),
        ("state-loan", "define actor A1 as state { int count = 0; } pub tick():void { handle_state(critical-write |s| -> { s.count += 1; }); } end"),
    ] {
        reject_both_modes(label, src);
    }
}

#[test]
fn tensor_autograd_and_actor_pipeline_fail_closed() {
    for (label, src) in [
        ("tensor", "define actor TensorStage as state { int version = 0; } end"),
        ("gradient", "define actor GradientStage as on #backward :: return; end end"),
        ("actor-pipe", "fnc main():int { spawn A() pipe spawn B(); return 42; }"),
        ("model", "define model LinearNet as Dense(2, 1) end"),
        ("await", "fnc main():int { await spawn Worker(); return 42; }"),
        ("borrow-escape", "define actor Stage as state { int weight = 2; } pub leak():int { return self.weight; } end"),
        ("actor-ref", "fnc main():int { let a = spawn Worker(); a.send(42); return 42; }"),
    ] {
        reject_both_modes(label, src);
    }
}

#[test]
fn words_inside_comments_do_not_disable_scalar_jit() {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-actor-comment-{}-{nonce}.ores",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "fnc main():int{return 6*7;} // define actor, trait, Tensor, autograd, pipe\n",
    ).unwrap();
    for mode in ["run", "ir"] {
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode).arg(&path).output().unwrap();
        assert!(result.status.success(), "{mode}: {}", String::from_utf8_lossy(&result.stderr));
        if mode == "run" {
            assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
        } else {
            assert!(!result.stdout.is_empty(), "IR mode must produce output");
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hand_derived_scalar_backprop_update_runs_as_real_native_jit() {
    // A shared i64 *arithmetic intersection*, not tensor allocation, autograd,
    // an actor trainer or a float64 ML optimizer. d(0.5*(w*x-t)^2)/dw = (w*x-t)*x
    // and learning rate is an integer. Other backends can reuse this exact
    // source with their own actual native compiler/runtime.
    let source = "fnc main(w:int,x:int,t:int,rate:int):int{return w-rate*(w*x-t)*x;}";
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-scalar-gradient-{}-{nonce}.ores", std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    for (args,expected) in [
        (["5","2","6","1"], "-3\n"),
        (["3","1","5","1"], "5\n"),
        (["0","3","6","1"], "18\n"),
        (["7","0","6","1"], "7\n"),
    ] {
        let result=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run").arg(&path).args(args).output().unwrap();
        assert!(result.status.success(), "{}",String::from_utf8_lossy(&result.stderr));
        assert_eq!(String::from_utf8_lossy(&result.stdout),expected);
    }
    let overflow=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run").arg(&path)
        .args(["9223372036854775807","2","0","1"]).output().unwrap();
    assert!(!overflow.status.success(),"checked numeric overflow must fail");
    assert!(overflow.stdout.is_empty(),"overflow emitted a result");
    let ir=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("ir").arg(&path).output().unwrap();
    assert!(ir.status.success(), "{}",String::from_utf8_lossy(&ir.stderr));
    assert!(!ir.stdout.is_empty(),"scalar native IR not emitted");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn identical_portable_scalar_step_executes_as_native_cranelift_guest() {
    const SOURCE: &str = include_str!("fixtures/actor-ml-scalar-step.ores");
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-shared-step-{}-{nonce}.ores", std::process::id()
    ));
    std::fs::write(&path, SOURCE).expect("materialize exact canonical fixture bytes");
    for mode in ["run", "ir"] {
        let result=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode).arg(&path).output().expect("execute real compiler");
        assert!(result.status.success(), "{mode}: {}",
            String::from_utf8_lossy(&result.stderr));
        if mode == "run" {
            assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
        } else {
            assert!(!result.stdout.is_empty(), "real CLIF IR required");
        }
    }
    std::fs::remove_file(path).expect("clean up source fixture");
}
