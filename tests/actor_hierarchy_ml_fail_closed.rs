//! Actor and differentiable pipeline syntax is outside Cranelift stage-0.
//! Execute the actual compiler in BOTH modes. A diagnostic-only denial is a
//! successful safety test, never actor or ML implementation evidence.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

// A predictable /tmp name must never overwrite an unrelated file or follow
// an attacker-planted symlink. Fixture content belongs to the private source
// checkout, so create each temporary file with restrictive permissions.
fn write_new_source(path: &std::path::Path, source: &str) {
    write_new_bytes(path, source.as_bytes());
}

fn write_new_bytes(path: &std::path::Path, source: &[u8]) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).expect("create exclusive source fixture");
    file.write_all(source).expect("write exact source fixture");
}

fn reject_both_modes(label: &str, source: &str) {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-actor-denial-{}-{}-{nonce}.ores",
        std::process::id(),
        label
    ));
    write_new_source(&path, source);
    for mode in ["run", "ir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&path)
            .output()
            .expect("invoke native compiler");
        assert!(
            !output.status.success(),
            "{label}/{mode} was silently admitted"
        );
        assert!(
            output.stdout.is_empty(),
            "{label}/{mode} emitted output or native IR"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.trim().is_empty(),
            "{label}/{mode} must diagnose rejection"
        );
        // Rejecting for missing files, CLI usage errors or runner/toolchain faults
        // is not proof that unsupported guest syntax was denied by the parser.
        assert!(
            [
                "orescranelift: DECL_KIND_UNSUPPORTED: ",
                "orescranelift: Cranelift source actor hierarchy, traits and mailbox runtime unsupported",
                "orescranelift: E_CRITICAL_UNSUPPORTED: ",
                "orescranelift: unexpected token at byte ",
                "orescranelift: unbound or unsupported identifier: ",
                "orescranelift: invalid character at byte ",
                "orescranelift: expected scalar expression at byte ",
            ]
            .iter()
            .any(|prefix| stderr.starts_with(prefix)),
            "{label}/{mode} rejected for an unrelated reason: {stderr}"
        );
        assert!(
            !stderr.contains("thread 'main' panicked"),
            "{label}/{mode} crashed: {stderr}"
        );
    }
    std::fs::remove_file(path).expect("remove test fixture");
}

#[test]
fn actor_class_hierarchy_and_protocol_fail_closed() {
    for (label, src) in [
        (
            "basic",
            "define actor A1 as end\nfnc main():int{return 42;}",
        ),
        (
            "canonical-counter",
            include_str!("fixtures/actor-counter-target.ores"),
        ),
        (
            "shared",
            "define shared actor A1 as end\nfnc main():int{return 42;}",
        ),
        (
            "inherited",
            "define actor Parent as end\ndefine actor Child extends Parent as end",
        ),
        ("class-as-actor", "define class Child extends Actor as end"),
        (
            "ordinary-class-parent",
            "define class Plain as end\ndefine actor Bad extends Plain as end",
        ),
        (
            "two-parents",
            "define actor A as end\ndefine actor B as end\ndefine actor C extends A, B as end",
        ),
        (
            "interface",
            "define interface Reader as read(): int; end\ndefine actor A implements Reader as end",
        ),
        (
            "trait",
            "define trait T as step(): void; end\ndefine actor A implements T as end",
        ),
        (
            "supervisor",
            "define supervisor S1 as end\ndefine actor A1 extends S1 as end",
        ),
        ("isolated", "define isoactor A1 as end"),
        ("untrusted", "define untrusted actor A1 as end"),
        (
            "runtime-method",
            "define actor A1 as pub tick(): void { return; } end",
        ),
        (
            "on-handler",
            "define actor A1 as on #tick :: return; end end",
        ),
        (
            "sealed-state",
            "define actor A1 as state { int count = 0; } end",
        ),
        (
            "state-loan",
            "define actor A1 as state { int count = 0; } pub tick():void { handle_state(critical-write |s| -> { s.count += 1; }); } end",
        ),
    ] {
        reject_both_modes(label, src);
    }
}

#[test]
fn tensor_autograd_and_actor_pipeline_fail_closed() {
    for (label, src) in [
        (
            "tensor",
            "define actor TensorStage as state { int version = 0; } end",
        ),
        (
            "gradient",
            "define actor GradientStage as on #backward :: return; end end",
        ),
        (
            "actor-pipe",
            "fnc main():int { spawn A() pipe spawn B(); return 42; }",
        ),
        ("model", "define model LinearNet as Dense(2, 1) end"),
        (
            "await",
            "fnc main():int { await spawn Worker(); return 42; }",
        ),
        (
            "borrow-escape",
            "define actor Stage as state { int weight = 2; } pub leak():int { return self.weight; } end",
        ),
        (
            "actor-ref",
            "fnc main():int { let a = spawn Worker(); a.send(42); return 42; }",
        ),
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
    write_new_source(
        &path,
        "fnc main():int{return 6*7;} // define actor, trait, Tensor, autograd, pipe\n",
    );
    for mode in ["run", "ir"] {
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
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
        "orescranelift-scalar-gradient-{}-{nonce}.ores",
        std::process::id()
    ));
    write_new_source(&path, source);
    for (args, expected) in [
        (["5", "2", "6", "1"], "-3\n"),
        (["3", "1", "5", "1"], "5\n"),
        (["0", "3", "6", "1"], "18\n"),
        (["7", "0", "6", "1"], "7\n"),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&result.stdout), expected);
    }
    let overflow = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .args(["9223372036854775807", "2", "0", "1"])
        .output()
        .unwrap();
    assert!(
        !overflow.status.success(),
        "checked numeric overflow must fail"
    );
    assert!(overflow.stdout.is_empty(), "overflow emitted a result");
    assert_eq!(
        String::from_utf8_lossy(&overflow.stderr).trim(),
        "orescranelift: i64 arithmetic overflow",
        "nonzero exit from an unrelated CLI or runner failure cannot prove checked overflow"
    );
    let ir = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("ir")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        ir.status.success(),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(!ir.stdout.is_empty(), "scalar native IR not emitted");
    assert!(
        String::from_utf8_lossy(&ir.stdout).contains("function "),
        "IR mode must emit actual Cranelift function text"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn identical_portable_scalar_step_executes_as_native_cranelift_guest() {
    const SOURCE: &str = include_str!("fixtures/actor-ml-scalar-step.ores");
    assert_eq!(
        SOURCE, "pub fnc main(): int { return 2 - 1 * (2 * 1 - 42) * 1; }\n",
        "exact shared source fixture bytes must not drift between toolchains"
    );
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-shared-step-{}-{nonce}.ores",
        std::process::id()
    ));
    write_new_source(&path, SOURCE);
    for mode in ["run", "ir"] {
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&path)
            .output()
            .expect("execute real compiler");
        assert!(
            result.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        if mode == "run" {
            assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
        } else {
            assert!(!result.stdout.is_empty(), "real CLIF IR required");
            assert!(
                String::from_utf8_lossy(&result.stdout).contains("function "),
                "real CLIF function definition required"
            );
        }
    }
    std::fs::remove_file(path).expect("clean up source fixture");
}

#[test]
fn signed_i64_boundaries_are_checked_in_native_execution() {
    let cases: [(&str, &str, &str, bool); 8] = [
        (
            "add-overflow",
            "fnc main(x:int):int{return x+1;}",
            "9223372036854775807",
            false,
        ),
        (
            "add-success",
            "fnc main(x:int):int{return x+1;}",
            "9223372036854775806",
            true,
        ),
        (
            "sub-overflow",
            "fnc main(x:int):int{return x-1;}",
            "-9223372036854775808",
            false,
        ),
        (
            "sub-success",
            "fnc main(x:int):int{return x-1;}",
            "-9223372036854775807",
            true,
        ),
        (
            "mul-overflow",
            "fnc main(x:int):int{return x*2;}",
            "9223372036854775807",
            false,
        ),
        ("mul-success", "fnc main(x:int):int{return x*2;}", "3", true),
        (
            "neg-overflow",
            "fnc main(x:int):int{return -x;}",
            "-9223372036854775808",
            false,
        ),
        ("neg-success", "fnc main(x:int):int{return -x;}", "1", true),
    ];
    for (label, source, arg, should_succeed) in cases {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-checked-i64-{}-{}-{nonce}.ores",
            std::process::id(),
            label
        ));
        write_new_source(&path, source);
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .arg(arg)
            .output()
            .expect("invoke scalar compiler");
        if should_succeed {
            assert!(
                result.status.success(),
                "{label}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let output = String::from_utf8_lossy(&result.stdout);
            let expected = match label {
                "add-success" => "9223372036854775807\n",
                "sub-success" => "-9223372036854775808\n",
                "mul-success" => "6\n",
                "neg-success" => "-1\n",
                _ => unreachable!(),
            };
            assert_eq!(output, expected, "{label}: wrong native result");
        } else {
            assert!(
                !result.status.success(),
                "{label}: overflow silently accepted"
            );
            assert!(
                result.stdout.is_empty(),
                "{label}: overflow emitted a result"
            );
            assert_eq!(
                String::from_utf8_lossy(&result.stderr).trim(),
                "orescranelift: i64 arithmetic overflow",
                "{label}: an unrelated crash must not count as overflow"
            );
        }
        std::fs::remove_file(path).expect("remove bounded scalar test source");
    }
}

#[cfg(unix)]
#[test]
fn cli_refuses_symlinked_source_even_when_target_is_regular() {
    use std::os::unix::fs::symlink;
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let original = std::env::temp_dir().join(format!(
        "orescranelift-source-original-{}-{nonce}.ores",
        std::process::id()
    ));
    let link = std::env::temp_dir().join(format!(
        "orescranelift-source-link-{}-{nonce}.ores",
        std::process::id()
    ));
    write_new_source(&original, "fnc main():int{return 42;}");
    symlink(&original, &link).expect("create symlink for refusal test");
    for mode in ["run", "ir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&link)
            .output()
            .expect("invoke CLI");
        assert!(
            !output.status.success(),
            "{mode}: symlinked source was admitted"
        );
        assert!(
            output.stdout.is_empty(),
            "{mode}: symlink produced guest output"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "),
            "{mode}: missing CLI diagnostic"
        );
    }
    std::fs::remove_file(link).expect("remove symlink");
    std::fs::remove_file(original).expect("remove original source");
}

#[test]
fn input_rejection_is_specific_and_emits_no_guest_output() {
    let cases = [
        ("invalid-utf8", vec![0xff_u8], "source is not valid UTF-8"),
        (
            "over-budget",
            vec![b' '; 1024 * 1024 + 1],
            "source exceeds 1 MiB limit",
        ),
        (
            "nul-source",
            b"fnc main():int{return 42;}\0".to_vec(),
            "source contains NUL byte",
        ),
    ];
    for (label, bytes, diagnostic) in cases {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-input-denial-{}-{}-{nonce}.ores",
            std::process::id(),
            label
        ));
        write_new_bytes(&path, &bytes);
        for mode in ["run", "ir"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .expect("invoke native scalar CLI");
            assert!(
                !output.status.success(),
                "{label}/{mode}: unsupported input was accepted"
            );
            assert!(
                output.stdout.is_empty(),
                "{label}/{mode}: emitted guest output or IR"
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stderr).trim(),
                format!("orescranelift: {diagnostic}"),
                "{label}/{mode}: unrelated failure cannot count as source denial"
            );
        }
        std::fs::remove_file(path).expect("remove test input");
    }
}
