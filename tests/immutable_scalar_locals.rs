//! Real native Cranelift JIT candidate probes for bounded scalar locals.
//! The presence of tests is not evidence of execution; source CI must run.
use std::io::Write;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);

fn guest(source: &str, extension: &str, args: &[&str]) -> Output {
    let sequence = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-local-{}-{sequence}.{extension}",
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).expect("private fixture create_new");
    file.write_all(source.as_bytes()).expect("write fixture");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .args(args)
        .output()
        .expect("execute real native Cranelift guest");
    std::fs::remove_file(path).expect("remove private guest source");
    output
}

fn accepts(source: &str, args: &[&str], expected: &str) {
    for extension in ["atto", "ores"] {
        let out = guest(source, extension, args);
        assert!(
            out.status.success(),
            "{extension} must execute: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
        assert!(
            out.stderr.is_empty(),
            "successful guest emitted diagnostics"
        );
    }
}

fn refuses(source: &str, args: &[&str], category: &str) {
    for extension in ["atto", "ores"] {
        let out = guest(source, extension, args);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{extension}: compiler rejection must exit(1)"
        );
        assert!(
            out.stdout.is_empty(),
            "rejected source produced guest stdout"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.starts_with("orescranelift: "), "{stderr}");
        assert!(stderr.contains(category), "expected {category}: {stderr}");
        assert!(
            !stderr.contains("panicked at"),
            "compiler panic is not rejection"
        );
    }
}

#[test]
fn local_immutable_ssa_executes_in_native_jit() {
    for (source, args, expected) in [
        ("fnc main():int{int a=7;return a*6;}", &[][..], "42\n"),
        (
            "fnc main():int{int a=7;val b=a+35;return b;}",
            &[][..],
            "42\n",
        ),
        (
            "fnc main():int{const a=7;const b=a*6;return b;}",
            &[][..],
            "42\n",
        ),
        ("fnc main():int{const int a=7;return a*6;}", &[][..], "42\n"),
        ("fnc main():int{const a:int=7;return a*6;}", &[][..], "42\n"),
        (
            "fnc main():int{val a=7 as int;return a*6;}",
            &[][..],
            "42\n",
        ),
        (
            "fnc main(x:int):int{val a=x+1;val b=a*2;return b;}",
            &["20"][..],
            "42\n",
        ),
        (
            "fnc main(a:int,b:int):int{val c=Integer.rem(a,b);return c+41;}",
            &["9", "4"][..],
            "42\n",
        ),
        (
            "fnc main():int{int x=42;int y=1;return x;}",
            &[][..],
            "42\n",
        ),
        (
            "fnc main():int{const x=-9223372036854775808;return x;}",
            &[][..],
            "-9223372036854775808\n",
        ),
        (
            "fnc main():int{int x=7;return x as int * 6;}",
            &[][..],
            "42\n",
        ),
    ] {
        accepts(source, args, expected);
    }
}

#[test]
fn invalid_local_declarations_fail_before_jit() {
    for (source, category) in [
        ("fnc main():int{int a=a;return a;}", "unbound"),
        ("fnc main():int{int a=b;int b=7;return a;}", "unbound"),
        ("fnc main():int{int a=7;val a=8;return a;}", "duplicate"),
        ("fnc main(x:int):int{int x=7;return x;}", "duplicate"),
        (
            "fnc main(x:int):int{const a=x;return a;}",
            "E_NON_CONST_INITIALIZER",
        ),
        (
            "fnc main(x:int):int{int a=x+1;val b=a*2;return b;}",
            "E_NON_CONST_INITIALIZER",
        ),
        (
            "fnc main(x:int):int{val a=x;const b=a;return b;}",
            "E_NON_CONST_INITIALIZER",
        ),
        (
            "fnc main():int{const a=9223372036854775807+1;return a;}",
            "E_CONST_OVERFLOW",
        ),
        (
            "fnc main():int{int a=1;a=2;return a;}",
            "E_IMMUTABLE_REBIND",
        ),
        ("fnc main():int{let a=1;return a;}", "E_LET_UNSUPPORTED"),
    ] {
        refuses(source, &[], category);
    }
}

#[test]
fn unused_runtime_initializers_still_trap_eagerly() {
    refuses(
        "fnc main():int{int unused=9223372036854775807+1;return 42;}",
        &[],
        "E_CONST_OVERFLOW",
    );
    refuses(
        "fnc main(x:int):int{val unused=x+1;return 42;}",
        &["9223372036854775807"],
        "overflow",
    );
    refuses(
        "fnc main():int{val unused=Integer.rem(9,0);return 42;}",
        &[],
        "divisor zero",
    );
}

#[test]
fn scalar_locals_do_not_bypass_ownership_or_actor_admission() {
    for source in [
        "fnc main():int{int x=7;return borrow! x;}",
        "fnc main():int{int x=7;return spawn Worker();}",
        "fnc main():int{int x=7;return x%3;}",
        "fnc main():int{int x=7;return match x over 7 -> 42 end;}",
        "fnc main():int{int x=7;return x as bool;}",
    ] {
        refuses(source, &[], "");
    }
}

#[test]
fn terminal_wildcard_scalar_alias_native_jit() {
    for source in [
        include_str!("../contracts/incoming-scalar-bindings-v1/incoming-alias-typed.atto"),
        "fnc main():int{return match 40 over on _ as const z -> z+2 end;}",
        "fnc main():int{return match 20 over on _ as rebind int z -> {z=z+1;z=z+1;return z+20;} end;}",
        "fnc main():int{return match 42 over on _ as rebind -> rebind end;}",
    ] { accepts(source, &[], "42\n"); }
    accepts("fnc main(z:int):int{return match z over on _ as rebind int z -> {z=z+2;return z+20;} end;}", &["20"], "42\n");
    for (source, diagnostic) in [
        ("fnc main():int{return match 42 over on _ as int z -> {z=0;return z;} end;}", "E_IMMUTABLE_REBIND"),
        ("fnc main():int{return match 42 over on _ as bool z -> 1 end;}", "E_TYPE"),
        ("fnc main():int{return match 42 over on _ as rebind rebind int z -> z end;}", "E_SYNTAX"),
        ("fnc main():int{return match 42 over on _ as mut z -> z end;}", "E_UNSUPPORTED"),
        ("fnc main():int{return match 42 over on _ as int z when true -> z end;}", "MATCH_DEFAULT_BACKEND_UNSUPPORTED"),
        ("fnc main():int{return match z over on _ as int z -> z end;}", "unbound"),
    ] { refuses(source, &[], diagnostic); }
    refuses("fnc main(x:int):int{return match x+1 over on _ as int z -> 42 end;}", &["9223372036854775807"], "overflow");
}
