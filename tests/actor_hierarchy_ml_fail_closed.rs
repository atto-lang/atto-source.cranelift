//! AttoLang actor/ML sources are explicitly unsupported in scalar Cranelift.
//! Negative source admission is not actor or machine-learning parity evidence.
#[path = "support/mod.rs"]
mod support;

fn reject_both_modes(label: &str, source: &str) {
    for mode in ["run", "ir"] {
        let output = support::run_source(mode, source.as_bytes(), &[]);
        assert_eq!(output.status.code(), Some(1), "{label}/{mode}: unsupported source admitted");
        assert!(output.stdout.is_empty(), "{label}/{mode}: guest output or IR emitted");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.starts_with("orescranelift: "),
            "{label}/{mode}: missing compiler diagnostic");
        assert!(!diagnostic.contains("panicked at") && !diagnostic.contains("stack backtrace"),
            "{label}/{mode}: compiler crashed: {diagnostic}");
    }
}

#[test]
fn actor_class_hierarchy_and_protocol_fail_closed() {
    for (label, src) in [
        ("basic", "define actor A1 as end\nfnc main():int{return 42;}"),
        ("canonical-counter", include_str!("fixtures/actor-counter-target.ores")),
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
    let source = "fnc main():int{return 6*7;} // define actor, trait, Tensor, autograd, pipe\n";
    for mode in ["run", "ir"] {
        let result = support::run_source(mode, source.as_bytes(), &[]);
        assert_eq!(result.status.code(), Some(0), "{mode}: native compiler rejected comments");
        assert!(result.stderr.is_empty());
        if mode == "run" {
            assert_eq!(result.stdout, b"42\n");
        } else {
            assert!(!result.stdout.is_empty(), "IR mode must produce CLIF");
        }
    }
}

#[test]
fn hand_derived_scalar_backprop_update_runs_as_real_native_jit() {
    // Integer scalar derivative arithmetic; NOT autograd/tensor execution.
    let source = "fnc main(w:int,x:int,t:int,rate:int):int{return w-rate*(w*x-t)*x;}";
    for (args, expected) in [
        (["5", "2", "6", "1"], "-3\n"),
        (["3", "1", "5", "1"], "5\n"),
        (["0", "3", "6", "1"], "18\n"),
        (["7", "0", "6", "1"], "7\n"),
    ] {
        let output = support::run_source("run", source.as_bytes(), &args);
        assert_eq!(output.status.code(), Some(0), "scalar native execution failed");
        assert_eq!(output.stdout, expected.as_bytes());
        assert!(output.stderr.is_empty());
    }
    let overflow = support::run_source("run", source.as_bytes(),
        &["9223372036854775807", "2", "0", "1"]);
    assert_eq!(overflow.status.code(), Some(1), "checked overflow must reject");
    assert!(overflow.stdout.is_empty());
    assert_eq!(overflow.stderr, b"orescranelift: i64 arithmetic overflow\n");
    let ir = support::run_source("ir", source.as_bytes(), &[]);
    assert_eq!(ir.status.code(), Some(0), "valid scalar source must emit IR");
    assert!(!ir.stdout.is_empty(), "expected native CLIF IR");
    assert!(ir.stderr.is_empty());
}

#[test]
fn identical_portable_scalar_step_executes_as_native_cranelift_guest() {
    const SOURCE: &str = include_str!("fixtures/actor-ml-scalar-step.ores");
    assert_eq!(
        SOURCE,
        "pub fnc main(): int { return 2 - 1 * (2 * 1 - 42) * 1; }\n",
        "exact shared source bytes must not drift between toolchains"
    );
    for mode in ["run", "ir"] {
        let output = support::run_source(mode, SOURCE.as_bytes(), &[]);
        assert_eq!(output.status.code(), Some(0), "{mode}: native compiler failed");
        assert!(output.stderr.is_empty());
        if mode == "run" {
            assert_eq!(output.stdout, b"42\n");
        } else {
            assert!(!output.stdout.is_empty(), "IR requires actual CLIF");
        }
    }
}
