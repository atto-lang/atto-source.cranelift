//! Real stage-0 scalar native-JIT acceptance, not ML tensor/autograd parity.
//! Use one bounded test harness for both IR and execution.
#[path = "support/mod.rs"]
mod support;

fn execute(source: &str, mode: &str) -> std::process::Output {
    support::run_source(mode, source.as_bytes(), &[])
}

#[test]
fn native_compiler_output_capture_is_bounded_and_refuses_truncation() {
    let oversize = vec![b'Q'; 65_536 + 8192];
    let (captured, truncated) = support::drain_bounded(&oversize[..]).unwrap();
    assert_eq!(captured.len(), 65_536);
    assert!(truncated, "oversized diagnostics must never count as valid output");
    let (exact, truncated) = support::drain_bounded(&oversize[..65_536]).unwrap();
    assert_eq!(exact.len(), 65_536);
    assert!(!truncated, "exact-size output is admissible");
    let (empty, truncated) = support::drain_bounded(&oversize[..0]).unwrap();
    assert!(empty.is_empty());
    assert!(!truncated);
}

#[test]
fn scalar_gradient_reference_runs_through_native_jit() {
    // A deliberately scalar training-gradient expression:
    // x=3, w=5, b=1, target=9; d/dw (wx+b-target)^2 = 2*(16-9)*3 = 42.
    // This is NOT a tensor gradient or autodiff runtime.
    let source = "fnc main():int{return 2*((5*3+1)-9)*3;}";
    let result = execute(source, "run");
    assert!(
        result.status.success(),
        "native scalar gradient rejected: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"42\n");
    assert!(result.stderr.is_empty());
}

#[test]
fn integer_training_primitives_compile_to_ir_and_execute() {
    // Portable integer-only subset of the OresLLM scalar fixture; these are
    // source-to-real-JIT assertions, NOT FP64 or autograd evidence.
    let cases = [
        (
            "gradient",
            "fnc main():int{return 2*((5*3+1)-9)*3;}",
            "42\n",
        ),
        ("associativity", "fnc main():int{return (7*8)-14;}", "42\n"),
        (
            "signed_subtraction",
            "fnc main():int{return 13-(-29);}",
            "42\n",
        ),
        (
            "zero_cancellation",
            "fnc main():int{return (3*14)+(0*7);}",
            "42\n",
        ),
    ];
    for (name, source, expected) in cases {
        let ir = execute(source, "ir");
        assert_eq!(
            ir.status.code(),
            Some(0),
            "{name}: compiler IR phase rejected source"
        );
        assert!(!ir.stdout.is_empty(), "{name}: empty compiler/JIT IR");
        assert!(
            ir.stderr.is_empty(),
            "{name}: unexpected compiler IR diagnostics"
        );
        let run = execute(source, "run");
        assert_eq!(run.status.code(), Some(0), "{name}: native JIT failed");
        assert_eq!(
            run.stdout,
            expected.as_bytes(),
            "{name}: wrong real native value"
        );
        assert!(
            run.stderr.is_empty(),
            "{name}: unexpected native diagnostics"
        );
    }
}

#[test]
fn unsupported_ml_features_fail_before_native_ir_or_execution() {
    let rejected = [
        ("float literal", "fnc main():int{return 1.5;}"),
        ("float result", "fnc main():float{return 1.0;}"),
        ("tensor constructor", "fnc main():int{return TensorF64(1);}"),
        (
            "tensor import",
            "import fnc add from './tensor'; fnc main():int{return 42;}",
        ),
        (
            "ML class",
            "define class TensorF64 as end fnc main():int{return 42;}",
        ),
        (
            "autograd helper",
            "fnc gradient(x:int):int{return x;} fnc main():int{return gradient(2);}",
        ),
        ("ownership", "fnc main():int{return rt borrow 42;}"),
        (
            "actor source",
            "define actor Trainer as end fnc main():int{return 42;}",
        ),
        ("async source", "pub async fnc main():int{return 42;}"),
    ];
    for (name, source) in rejected {
        for mode in ["ir", "run"] {
            let result = execute(source, mode);
            assert_eq!(
                result.status.code(),
                Some(1),
                "{name} {mode}: unsupported source must be a compiler rejection"
            );
            assert!(
                result.stdout.is_empty(),
                "{name} {mode}: unsupported source emitted native IR or a guest result"
            );
            let diagnostics = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostics.starts_with("orescranelift: "),
                "{name} {mode}: compiler rejected without structured frontend diagnostic: {diagnostics}"
            );
            assert!(
                !diagnostics.contains("panicked at"),
                "{name} {mode}: compiler panicked: {diagnostics}"
            );
        }
    }
}

#[test]
fn checked_overflow_occurs_in_native_execution_not_ir_acceptance() {
    let source = "fnc main():int{return (9223372036854775807+1)*0;}";
    let ir = execute(source, "ir");
    assert!(
        ir.status.success(),
        "valid source must lower to native IR before runtime overflow: {}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(
        !ir.stdout.is_empty(),
        "IR emission must produce real Cranelift IR, not silently skip"
    );
    let runtime = execute(source, "run");
    assert_eq!(
        runtime.status.code(),
        Some(1),
        "checked overflow must be a native runtime error, not crash/signal"
    );
    assert!(
        runtime.stdout.is_empty(),
        "overflow published a native result"
    );
    assert_eq!(
        runtime.stderr, b"orescranelift: i64 arithmetic overflow\n",
        "exact native checked-overflow diagnostic must not be confused with infrastructure failure"
    );
}
