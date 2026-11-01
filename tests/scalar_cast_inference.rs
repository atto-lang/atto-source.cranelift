use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_source(source: &str, mode: &str, args: &[&str]) -> std::process::Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-cast-{}-{id}.ores",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .expect("start Cranelift CLI");
    std::fs::remove_file(path).expect("remove fixture");
    output
}

fn succeeds(source: &str, args: &[&str], expected: &str) {
    let output = run_source(source, "run", args);
    assert!(
        output.status.success(),
        "source failed: {source}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
}

fn rejects(source: &str, args: &[&str], diagnostic: Option<&str>) {
    let output = run_source(source, "run", args);
    assert!(!output.status.success(), "invalid source executed: {source}");
    assert!(output.stdout.is_empty(), "invalid source emitted stdout");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.is_empty(), "rejection must have diagnostic");
    if let Some(diagnostic) = diagnostic {
        assert!(
            stderr.contains(diagnostic),
            "wrong diagnostic for {source}: {stderr}"
        );
    }
}

#[test]
fn runtime_const_val_and_inferred_int_bindings_execute_in_real_jit() {
    for (source, args, expected) in [
        ("fnc main(a:int):int{const x=a+1;return x;}", &["41"][..], "42\n"),
        ("fnc main(a:int):int{int x=a+1;const y=x;return y;}", &["41"][..], "42\n"),
        ("fnc main(a:int):int{val x=a+1;return x;}", &["41"][..], "42\n"),
        ("fnc main():int{const x=7 as int;return x+35;}", &[][..], "42\n"),
        ("fnc main():int{const int x=7 as int;return x+35;}", &[][..], "42\n"),
        ("fnc main():int{const x:int=7 as int;return x+35;}", &[][..], "42\n"),
        ("fnc main():int{int x=7 as int;return x+35;}", &[][..], "42\n"),
        ("fnc main():int{val x=7 as int;return x+35;}", &[][..], "42\n"),
        ("fnc main():int{const x=(2+5) as int;return x+35;}", &[][..], "42\n"),
        ("fnc main():int{const x=7 as int as int;return x+35;}", &[][..], "42\n"),
        ("fnc main():int{return (40+2) as int;}", &[][..], "42\n"),
        ("fnc main(a:int):int{return (a+1) as int;}", &["41"][..], "42\n"),
        ("fnc main():int{const x=(-9223372036854775808) as int;return x;}", &[][..], "-9223372036854775808\n"),
        ("fnc main():int{const x=9223372036854775807 as int;return x;}", &[][..], "9223372036854775807\n"),
        ("fnc main():int{const x=Integer.bitXor(5,3) as int;return x+36;}", &[][..], "42\n"),
        ("fnc main(a:int):int{int x=a;const y=x;val z=y as int;return z+1;}", &["41"][..], "42\n"),
        ("fnc main(a:int):int{const unused=a+1;return 7;}", &["1"][..], "7\n"),
    ] {
        succeeds(source, args, expected);
    }
}

#[test]
fn checked_cast_and_mutability_gaps_fail_closed_before_native_result() {
    for (source, diagnostic) in [
        ("fnc main():int{return 7 as bool;}", Some("impossible Cranelift cast: int as bool")),
        ("fnc main():int{return 7 as string;}", Some("impossible Cranelift cast: int as string")),
        ("fnc main():int{return 7 as String;}", Some("impossible Cranelift cast: int as string")),
        ("fnc main():int{return 7 as? int;}", Some("Cranelift as? requires Option<T> checked-cast lowering")),
        ("fnc main():int{return 7 as Phantom;}", Some("nominal/dynamic casts unsupported")),
        ("fnc main():int{return 7 as int * 6;}", Some("arithmetic after checked cast requires parentheses")),
        ("fnc main():int{let int x=7;return x;}", Some("Cranelift mutable let lowering is not implemented")),
        ("fnc main():int{let x=7;return x;}", Some("Cranelift mutable let lowering is not implemented")),
        ("fnc main():int{const x=7 as int;x=8;return x;}", None),
        ("fnc main():int{val x=7;x=8;return x;}", None),
    ] {
        rejects(source, &[], diagnostic);
    }

    // Runtime-dependent const remains eager and checked: accepting the binding
    // must not turn overflow into dead-code elimination.
    rejects(
        "fnc main(a:int):int{const unused=a+1;return 7;}",
        &["9223372036854775807"],
        Some("i64 arithmetic overflow"),
    );
}

#[test]
fn identity_cast_does_not_add_a_cranelift_conversion() {
    let plain = run_source("fnc main():int{return 42;}", "ir", &[]);
    let casted = run_source("fnc main():int{return 42 as int;}", "ir", &[]);
    assert!(plain.status.success(), "plain IR failed");
    assert!(casted.status.success(), "cast IR failed");
    assert_eq!(
        plain.stdout, casted.stdout,
        "identity checked cast must not emit a conversion or bitcast"
    );
}
