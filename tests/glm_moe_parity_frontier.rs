//! Honest GLM/MoE cross-backend frontier for the stage-0 Cranelift JIT.
//! These are executable positive and explicit unsupported-source tests.
//! They do not prove that the shared 20-program GLM corpus compiles here.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_guest(mode: &str, source: &str, args: &[&str]) -> Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-cranelift-glm-{}-{sequence}.ores",
        std::process::id()
    ));
    // Distinct paths per concurrent invocation; no preexisting path overwrite.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    file.write_all(source.as_bytes()).unwrap();
    drop(file);
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    result
}

fn expect_native(source: &str, args: &[&str], expected: &str) {
    let result = run_guest("run", source, args);
    assert!(result.status.success(), "native source should succeed: {}",
        String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), expected);
    assert!(result.stderr.is_empty(), "successful guest emitted diagnostics");
}

fn expect_unsupported(source: &str) {
    for mode in ["run", "ir"] {
        let result = run_guest(mode, source, &[]);
        assert!(!result.status.success(), "unsupported GLM source admitted via {mode}");
        assert!(result.stdout.is_empty(), "unsupported source printed output via {mode}");
        assert!(!result.stderr.is_empty(), "rejection lacks diagnostic via {mode}");
    }
}

#[test]
fn scalar_top1_score_difference_intersection_executes_as_native_jit() {
    expect_native("fnc main(a:int,b:int):int{return a-b;}", &["9", "2"], "7\n");
    expect_native("fnc main(a:int,b:int):int{return a-b;}", &["-5", "-1"], "-4\n");
}

#[test]
fn scalar_assignment_total_arithmetic_intersection_executes_as_native_jit() {
    expect_native("fnc main(a:int,b:int,tokens:int,k:int):int{return a+b-tokens*k;}",
        &["2", "2", "4", "1"], "0\n");
    expect_native("fnc main(a:int,b:int,tokens:int,k:int):int{return a+b-tokens*k;}",
        &["1", "1", "4", "1"], "-2\n");
}

#[test]
fn scalar_learning_step_arithmetic_intersection_executes_as_native_jit() {
    expect_native("fnc main(prior:int):int{return prior+1;}", &["3"], "4\n");
    expect_native("fnc main(prior:int):int{return prior+1;}",
        &["9223372036854775806"], "9223372036854775807\n");
}

#[test]
fn checked_overflow_rejects_before_claiming_glm_guest_success() {
    let result = run_guest("run", "fnc main(prior:int):int{return prior+1;}",
        &["9223372036854775807"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!result.stderr.is_empty());
}

#[test]
fn helper_functions_in_shared_glm_corpus_are_not_supported() {
    expect_unsupported("fnc route(a:int,b:int):int{return a-b;} pub fnc main():int{return route(9,2);}");
}

#[test]
fn deterministic_top1_control_flow_is_not_yet_lowered() {
    expect_unsupported("pub fnc main():int{if (9>=2) {return 42;} fi return 1;}");
}

#[test]
fn boolean_capacity_and_cancellation_logic_must_reject() {
    expect_unsupported("pub fnc main():int{if (true && false) {return 1;} fi return 42;}");
}

#[test]
fn float_fixed_gate_prediction_must_not_silently_become_integer_arithmetic() {
    expect_unsupported("pub fnc main():int{if (0.75*2.0==1.5) {return 42;} fi return 1;}");
}

#[test]
fn actor_ownership_and_suspending_source_cannot_bypass_stage_zero() {
    expect_unsupported("define isolated actor Gate as state int value; end pub fnc main():int{return 42;}");
}

#[test]
fn cross_file_imports_are_not_flattened_into_an_unreviewed_program() {
    expect_unsupported("import fnc route from \"./other.ores\"; pub fnc main():int{return route();}");
}

#[test]
fn nonzero_undocumented_operators_do_not_gain_fake_glm_feature_support() {
    expect_unsupported("pub fnc main():int{return 2 ^ 3;}");
    expect_unsupported("pub fnc main():int{return 3 / 2;}");
}
