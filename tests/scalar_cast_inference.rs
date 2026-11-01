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
        ("fnc main():int{let x=7;x=42;return x;}", &[][..], "42\n"),
        ("fnc main():int{let int x=41;x=x+1;return x;}", &[][..], "42\n"),
        ("fnc main():int{let x:int=41;x=x+1;return x;}", &[][..], "42\n"),
        ("fnc main():int{let x=7 as int;x=42 as int;return x;}", &[][..], "42\n"),
        ("fnc main():int{let x=7;val snap=x;x=42;return snap+35;}", &[][..], "42\n"),
        ("fnc main():int{let x=7;const snap=x;x=42;return snap+35;}", &[][..], "42\n"),
        ("fnc main():int{const base=7;let x=base;x=x+35;return x;}", &[][..], "42\n"),
        ("fnc main(a:int):int{let x=a;x=x+1;return x;}", &["41"][..], "42\n"),
        ("fnc main(a:int):int{let x=a;const snap=x;x=x+1;return snap;}", &["42"][..], "42\n"),
        ("fnc main():int{let x=7;x=8;x=42;return x;}", &[][..], "42\n"),
        ("fnc main():int{let x=7;x=x+1;x=x+34;return x;}", &[][..], "42\n"),
        ("fnc main():int{let x=7;const y=x as int;x=42;return y+35;}", &[][..], "42\n"),
        ("fnc main():int{let x=7;val y=x;x=42;return x;}", &[][..], "42\n"),
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
        ("fnc main():int{const x=7;x=8;return x;}", Some("cannot reassign immutable scalar local")),
        ("fnc main():int{val x=7;x=8;return x;}", Some("cannot reassign immutable scalar local")),
        ("fnc main():int{int x=7;x=8;return x;}", Some("cannot reassign immutable scalar local")),
        ("fnc main():int{let x=7;let x=8;return x;}", Some("invalid or duplicate immutable local")),
        ("fnc main():int{let x=x+1;return x;}", Some("unbound or unsupported identifier")),
        ("fnc main():int{let x=7;x=missing;return x;}", Some("unbound or unsupported identifier")),
        ("fnc main():int{let x=7;x=true;return x;}", Some("unbound or unsupported identifier")),
        ("fnc main():int{let x=7;x=7 as bool;return x;}", Some("impossible Cranelift cast: int as bool")),
        ("fnc main():int{let x=7;x=7 as? int;return x;}", Some("Cranelift as? requires Option<T> checked-cast lowering")),
        ("fnc main():int{let x=7;x=7 as Phantom;return x;}", Some("nominal/dynamic casts unsupported")),
        ("fnc main():int{const as=7;return as;}", Some("reserved scalar binding name")),
        ("fnc main():int{const bool=7;return bool;}", Some("reserved scalar binding name")),
        ("fnc main():int{const true=7;return true;}", Some("reserved scalar binding name")),
        ("fnc main():int{let val=7;return val;}", Some("reserved scalar binding name")),
        ("fnc main():int{let rt=7;return rt;}", Some("reserved scalar binding name")),
        ("fnc main(as:int):int{return as;}", Some("reserved scalar binding name")),
        ("fnc main(bool:int):int{return bool;}", Some("reserved scalar binding name")),
        ("fnc main():int{const x=7;let x=8;return x;}", Some("invalid or duplicate immutable local")),
        ("fnc main():int{const x=7;x=8 as int;return x;}", Some("cannot reassign immutable scalar local")),
        ("fnc main():int{const x=7 as int;x=8;return x;}", None),
        ("fnc main():int{val x=7;x=8;return x;}", None),
    ] {
        rejects(source, &[], diagnostic);
    }

    // Reassignments are eager even when their resulting SSA value is unused.
    rejects(
        "fnc main(a:int):int{let x=7;x=a+1;return 42;}",
        &["9223372036854775807"],
        Some("i64 arithmetic overflow"),
    );
    // Immutable snapshots must not retroactively observe a later let write.
    succeeds(
        "fnc main():int{let x=7;const old=x;x=42;return old+35;}",
        &[],
        "42\n",
    );

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
