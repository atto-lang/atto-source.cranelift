use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

// Cargo runs integration tests in parallel. Never let fixtures overwrite or
// unlink each other's source files: doing so masks compiler outcomes.
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn fixture(source: &str, args: &[&str], ok: bool, expected: &str) {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-{}-{}-{}.ores",
        std::process::id(),
        sequence,
        source.len()
    ));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .args(args)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        output.status.success(),
        ok,
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if ok {
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    } else {
        assert!(
            output.stdout.is_empty(),
            "rejected compilation printed stdout"
        );
        assert!(!output.stderr.is_empty(), "rejection requires a diagnostic");
    }
}
#[test]
fn native_execution_and_negative_controls() {
    fixture("fnc main():int{return 2+3*4;}", &[], true, "14\n");
    fixture(
        "fnc main(x:int,y:int):int{return x*3+y;}",
        &["7", "8"],
        true,
        "29\n",
    );
    fixture(
        "fnc main(x:int):int{return x+1;}",
        &["9223372036854775807"],
        false,
        "",
    );
    fixture(
        "fnc main(x:int):int{return x-1;}",
        &["-9223372036854775808"],
        false,
        "",
    );
    fixture(
        "fnc main(x:int):int{return x*2;}",
        &["4611686018427387904"],
        false,
        "",
    );
    fixture(
        "fnc main():int{return -9223372036854775808;}",
        &[],
        true,
        "-9223372036854775808\n",
    );
    fixture("fnc main():int{return 1/0;}", &[], false, "");
    fixture("fnc main():int{return 4;} // café", &[], true, "4\n");
    fixture("fnc main():int{return 1;} unsafe", &[], false, "");
}

#[test]
fn native_integer_bitxor_source_execution() {
    // Real Oreslang-shaped source -> frontend -> CLIF native JIT -> result.
    // Signed64 edge vectors correspond to Java strict named arithmetic v1.
    fixture(
        "fnc main():int{return Integer.bitXor(0,0);}",
        &[],
        true,
        "0\n",
    );
    fixture(
        "fnc main():int{return Integer.bitXor(5,3);}",
        &[],
        true,
        "6\n",
    );
    fixture(
        "fnc main():int{return Integer.bitXor(-1,0);}",
        &[],
        true,
        "-1\n",
    );
    fixture(
        "fnc main():int{return Integer.bitXor(-9223372036854775808,9223372036854775807);}",
        &[],
        true,
        "-1\n",
    );
    fixture(
        "fnc main(x:int,y:int):int{return Integer.bitXor(x+1,y*2);}",
        &["4", "3"],
        true,
        "3\n",
    );
    // Arity, unsupported builtins, and the removed XOR operator fail closed.
    fixture("fnc main():int{return Integer.bitXor(1);}", &[], false, "");
    fixture(
        "fnc main():int{return Integer.bitXor(1,2,3);}",
        &[],
        false,
        "",
    );
    fixture("fnc main():int{return Integer.rem(9,4);}", &[], false, "");
    fixture("fnc main():int{return 5 ^ 3;}", &[], false, "");
    fixture(
        "fnc main():int{return Integer.bitXor(7%3,Integer.bitXor(4,2));}",
        &[],
        false,
        "",
    );
    for expression in [
        "Integer.bitXor(9223372036854775807+1,0)",
        "Integer.bitXor(0,9223372036854775807+1)",
        "Integer.bitXor((9223372036854775807+1)%-1,0)",
        "7%Integer.bitXor(1,2)",
    ] {
        fixture(
            &format!("fnc main():int{{return {expression};}}"),
            &[],
            false,
            "",
        );
    }
}

#[test]
fn native_removed_percent_operator_fails_closed() {
    for source in [
        "fnc main():int{return -7%3;}",
        "fnc main():int{return 7%-3;}",
        "fnc main():int{return -7%-3;}",
        "fnc main():int{return 8%3*2;}",
        "fnc main():int{return 20%6%3;}",
        "fnc main(x:int):int{return x%-1;}",
        "fnc main(x:int):int{return x%0;}",
        "fnc main(x:int,y:int):int{return x%y;}",
        "fnc main(x:int):int{return x%(1+1);}",
        "fnc main():int{const unused=7%3; return 42;}",
    ] {
        fixture(source, &[], false, "");
    }
    // Comments do not participate in lexical operator admission.
    fixture(
        "fnc main():int{return 42;} // 7%3 is commented out",
        &[],
        true,
        "42\n",
    );
}

#[test]
fn forge_scalar_v1_intersection() {
    // The 6 positive and 5 negative sources from Forge scalar/v1.
    // The actual shared-fixture blob comparison remains a separate parity gate.
    for (source, expected) in [
        ("pub fnc main(): int { return 42; }", "42\n"),
        ("pub fnc main(): int { return 4 + 6 * (5 - 1); }", "28\n"),
        ("pub fnc main(): int { return -(7 * 3); }", "-21\n"),
        (
            "pub fnc main(): int { return -9223372036854775808; }",
            "-9223372036854775808\n",
        ),
        (
            "pub fnc main(): int { return 9223372036854775807; }",
            "9223372036854775807\n",
        ),
        (
            "pub fnc main(): int { return 9223372036854775807 - 1; }",
            "9223372036854775806\n",
        ),
    ] {
        fixture(source, &[], true, expected);
    }
    for source in [
        "pub fnc main(): int { return undeclared_value; }",
        "pub fnc main(): int { return 3 / 2; }",
        "define actor Worker as { pub fnc on_message(int n): int { return n; } } pub fnc main(): int { const worker = spawn Worker(); stop worker; return 43; }",
        "pub async fnc main(): int { return 42; }",
        "pub fnc main(): int { return match 2 over on 2 { 42; } end; }",
    ] {
        fixture(source, &[], false, "");
    }
}

#[test]
fn java_scalar_grammar_and_integer_boundaries() {
    fixture(
        "fnc main(int x, y:int):int{return x*1_000+y;}",
        &["4", "2"],
        true,
        "4002\n",
    );
    fixture(
        "fnc main():int{return -9_223_372_036_854_775_808;}",
        &[],
        true,
        "-9223372036854775808\n",
    );
    fixture(
        "fnc main():int{return 9_223_372_036_854_775_807;}",
        &[],
        true,
        "9223372036854775807\n",
    );
    fixture(
        "fnc main():int{return -09223372036854775808;}",
        &[],
        false,
        "",
    );
    // The separator path must preserve native arithmetic overflow checks.
    fixture(
        "fnc main(int x):int{return x+1_000;}",
        &["9223372036854775807"],
        false,
        "",
    );
    for source in [
        "fnc main():int{return 1__0;}",
        "fnc main():int{return 1_;}",
        "fnc main():int{return -9__223_372_036_854_775_808;}",
        "fnc main():int{return 9_223_372_036_854_775_808;}",
        "fnc main(int x,x:int):int{return x;}",
        "fnc main(int x,int y,int z,int w,int v):int{return x;}",
        "fnc main(int return):int{return return;}",
        "fnc main(unsafe:int):int{return unsafe;}",
        "fnc main():int{return unsafe;}",
    ] {
        fixture(source, &[], false, "");
    }
    fixture(
        "fnc main():int{return 42;} // unsafe is text in this comment",
        &[],
        true,
        "42\n",
    );
}

#[cfg(unix)]
#[test]
fn source_path_fifo_and_symlink_admission_never_blocks() {
    use std::ffi::CString;
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    // Real CLI invocation is required; a parser unit test cannot detect a
    // blocking open, which happens before the parser sees a single byte.
    let nonce = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let fifo =
        std::env::temp_dir().join(format!("orescranelift-fifo-{}-{nonce}", std::process::id()));
    let link = fifo.with_extension("link");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: the CString provides a valid NUL-terminated POSIX pathname.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    symlink(&fifo, &link).unwrap();

    for source in [&fifo, &link] {
        for mode in ["run", "ir"] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(source)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("{mode} blocked on non-regular source: {}", source.display());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let result = child.wait_with_output().unwrap();
            assert!(
                !result.status.success(),
                "{mode} admitted non-regular source"
            );
            assert!(
                result.stdout.is_empty(),
                "{mode} emitted stdout on rejection"
            );
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("source must be a regular file"),
                "{mode} returned wrong diagnostic: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
    std::fs::remove_file(link).unwrap();
    std::fs::remove_file(fifo).unwrap();
}

#[test]
fn immutable_default_scalar_jit_parity_spellings() {
    // All forms execute real compiled Cranelift native code, never an
    // independent Rust expression evaluator.
    for declaration in [
        "int x = 7;",
        "const x = 7;",
        "const int x = 7;",
        "const x: int = 7;",
    ] {
        fixture(
            &format!("fnc main():int{{{declaration} return x;}}"),
            &[],
            true,
            "7\n",
        );
    }
    for (source, args, result) in [
        (
            "fnc main(int a):int{int x=a+2; int y=x*3; return y;}",
            &["5"][..],
            "21\n",
        ),
        (
            "fnc main(x:int):int{int x2 = x*2; int y=x2+1; return y;}",
            &["20"][..],
            "41\n",
        ),
        (
            "fnc main():int{const x=1+2*3; const y=x+4; return x+y;}",
            &[][..],
            "18\n",
        ),
        (
            "fnc main():int{const value = -9223372036854775808; return value;}",
            &[][..],
            "-9223372036854775808\n",
        ),
        (
            "fnc main():int{const x=7; const y=x*3; return y;}",
            &[][..],
            "21\\n",
        ),
        (
            "fnc main():int{const x=7; return Integer.bitXor(x,3);}",
            &[][..],
            "4\n",
        ),
    ] {
        fixture(source, args, true, result);
    }
}

#[test]
fn immutable_default_bindings_fail_closed_on_mutation_aliases_and_inference() {
    for source in [
        "fnc main():int{int x=7; x=8; return x;}",
        "fnc main():int{const x=7; x=8; return x;}",
        "fnc main():int{const int x=7; x+=1; return x;}",
        "fnc main():int{const x: int=7; x=x+1; return x;}",
        "fnc main():int{int x=7; rt mut borrow x; return x;}",
        "fnc main():int{int x=7; const x=8; return x;}",
        "fnc main(int x):int{const x=8; return x;}",
        "fnc main():int{int x=x+1; return x;}",
        "fnc main():int{int x=y; int y=7; return x;}",
        "fnc main():int{const x=missing; return x;}",
        "fnc main():int{const x; return 7;}",
        "fnc main():int{int x; return 7;}",
        "fnc main():int{string x=7; return x;}",
        "fnc main():int{const x: bool=7; return x;}",
        "fnc main():int{const x=7; let int y=8; return x;}",
        "fnc main():int{val int x=7; return x;}",
        "fnc main():int{int x=7; int x=8; return x;}",
        "fnc main():int{const const x=7; return x;}",
        "fnc main():int{const x=7; return x=8;}",
        "fnc main():int{int x=7; return x+unknown;}",
        "fnc main():int{const x=7; return x();}",
        "fnc main():int{int x=7; return x as float;}",
    ] {
        fixture(source, &[], false, "");
    }
    // Standalone const must never accept runtime-dependent expressions, even
    // when the expression is otherwise readonly and inferable as signed64.
    for source in [
        "fnc main(int a):int{const x=a+1; return x;}",
        "fnc main():int{int x=7; const y=x+1; return y;}",
        "fnc main():int{const x=9223372036854775807+1; return x;}",
    ] {
        fixture(source, &["1"][..0], false, "");
    }
    // A never-read initializer must still run its checked computation.
    fixture(
        "fnc main(x:int):int{int unused=x+1; return 7;}",
        &["9223372036854775807"],
        false,
        "",
    );
    fixture(
        "fnc main():int{int unused=9223372036854775807+1; return 7;}",
        &[],
        false,
        "",
    );
}

#[test]
fn immutable_default_local_budget_and_name_resolution() {
    let mut within_limit = String::from("fnc main():int{");
    for i in 0..64 {
        within_limit.push_str(&format!("int v{i}={i};"));
    }
    within_limit.push_str("return v63;}");
    fixture(&within_limit, &[], true, "63\n");

    let mut over_limit = String::from("fnc main():int{");
    for i in 0..65 {
        over_limit.push_str(&format!("int v{i}={i};"));
    }
    over_limit.push_str("return v64;}");
    fixture(&over_limit, &[], false, "");

    // Confirm commented-out declarations do not become live bindings.
    fixture(
        "fnc main():int{// const x=9;\n int x=7; return x;}",
        &[],
        true,
        "7\n",
    );
    fixture(
        "fnc main():int{// const x=9;\n return x;}",
        &[],
        false,
        "",
    );
}
