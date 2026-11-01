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
        true,
        "7\n",
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
fn native_signed_remainder_literal_divisor() {
    for (source, args, expected) in [
        ("fnc main():int{return -7%3;}", &[][..], "-1\n"),
        ("fnc main():int{return 7%-3;}", &[][..], "1\n"),
        ("fnc main():int{return -7%-3;}", &[][..], "-1\n"),
        ("fnc main():int{return 8%3*2;}", &[][..], "4\n"),
        ("fnc main():int{return 20%6%3;}", &[][..], "2\n"),
        (
            "fnc main(x:int):int{return x%-1;}",
            &["-9223372036854775808"][..],
            "0\n",
        ),
        (
            "fnc main(x:int):int{return x%-9223372036854775808;}",
            &["-9223372036854775808"][..],
            "0\n",
        ),
        (
            "fnc main(x:int):int{return x%-9223372036854775808;}",
            &["9223372036854775807"][..],
            "9223372036854775807\n",
        ),
        ("fnc main():int{return 8-7%3*2;}", &[][..], "6\n"),
    ] {
        fixture(source, args, true, expected);
    }
    // A -1 divisor must not erase a checked overflow in its numerator.
    fixture(
        "fnc main(x:int):int{return (x+1)%-1;}",
        &["9223372036854775807"],
        false,
        "",
    );
    for (source, args) in [
        ("fnc main(x:int):int{return x%0;}", &["7"][..]),
        ("fnc main(x:int,y:int):int{return x%y;}", &["7", "2"][..]),
        ("fnc main(x:int):int{return x%(1+1);}", &["7"][..]),
        ("fnc main(x:int):int{return x%-x;}", &["7"][..]),
        (
            "fnc main(x:int):int{return x%-9223372036854775809;}",
            &["7"][..],
        ),
        ("fnc main(x:int):int{return x/2;}", &["7"][..]),
    ] {
        fixture(source, args, false, "");
    }
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


// Additional compiler-to-native-JIT tests: these check actual Oreslang guest
// execution and fail-closed negative behavior, not just parser acceptance.
#[test]
fn native_integer_precision_matrix_without_f64_conversion() {
    for (source, expected) in [
        (
            "fnc main():int{return 9007199254740993-9007199254740992;}",
            "1\n",
        ),
        (
            "fnc main():int{return -9007199254740993-(-9007199254740992);}",
            "-1\n",
        ),
        (
            "fnc main():int{return 9223372036854775807-9223372036854775806;}",
            "1\n",
        ),
        (
            "fnc main():int{return -9223372036854775808+1;}",
            "-9223372036854775807\n",
        ),
        (
            "fnc main():int{return 1_000_000_000_000_001-1_000_000_000_000_000;}",
            "1\n",
        ),
        ("fnc main():int{return -(-42);}", "42\n"),
    ] {
        fixture(source, &[], true, expected);
    }
}

#[test]
fn native_integer_operator_precedence_matrix() {
    for (source, expected) in [
        ("fnc main():int{return 20-5-3;}", "12\n"),
        ("fnc main():int{return 2+3*4;}", "14\n"),
        ("fnc main():int{return (2+3)*4;}", "20\n"),
        ("fnc main():int{return -7*6;}", "-42\n"),
        ("fnc main():int{return 4*3-2*5;}", "2\n"),
        ("fnc main():int{return (6-2)*(-7);}", "-28\n"),
    ] {
        fixture(source, &[], true, expected);
    }
}

#[test]
fn native_i64_checked_runtime_overflow_matrix() {
    for (source, args) in [
        ("fnc main(x:int):int{return x+1;}", &["9223372036854775807"][..]),
        ("fnc main(x:int):int{return x-1;}", &["-9223372036854775808"][..]),
        ("fnc main(x:int):int{return -x;}", &["-9223372036854775808"][..]),
        ("fnc main(x:int):int{return x*2;}", &["4611686018427387904"][..]),
        ("fnc main(x:int):int{return x*x;}", &["3037000500"][..]),
        (
            "fnc main(x:int):int{return Integer.bitXor(x+1,0);}",
            &["9223372036854775807"][..],
        ),
    ] {
        fixture(source, args, false, "");
    }
}

#[test]
fn syntax_rejections_produce_no_guest_stdout() {
    for source in [
        "fnc main():int{return 4^1;}",
        "fnc main():int{return 4^^1;}",
        "fnc main():int{return 4/2;}",
        "fnc main():int{return 4.0;}",
        "fnc main():float{return 4;}",
        "fnc main():int{return Integer.pow(3,4);}",
        "fnc main():int{return Integer.bitXor(3);}",
        "fnc main():int{return 4;} fnc another():int{return 6;}",
        "fnc main():int{return 4;} // comment\nunsafe",
    ] {
        fixture(source, &[], false, "");
    }
}

#[test]
fn native_entry_parameter_count_and_i64_arguments_fail_closed() {
    fixture("fnc main(x:int):int{return x;}", &[], false, "");
    fixture(
        "fnc main(x:int):int{return x;}",
        &["9223372036854775808"],
        false,
        "",
    );
    fixture(
        "fnc main(x:int):int{return x;}",
        &["-9223372036854775809"],
        false,
        "",
    );
    fixture(
        "fnc main(x:int):int{return x;}",
        &["not-an-int"],
        false,
        "",
    );
    fixture(
        "fnc main(x:int):int{return x;}",
        &["42", "43"],
        false,
        "",
    );
    fixture(
        "fnc main(x:int):int{return x;}",
        &["42"],
        true,
        "42\n",
    );
}
