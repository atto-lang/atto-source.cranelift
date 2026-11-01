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
        assert!(
            output.stderr.is_empty(),
            "successful native JIT execution emitted stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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

// Real native Cranelift JIT guest execution of the shared scalar expressions.
// Backend subset deliberately excludes actors, imports, control flow and classes.
#[test]
fn shared_five_backend_scalar_wave_native_jit() {
    // The bytes supplied to the JIT are those of the shared native .ores
    // corpus, including its two comment lines and final LF. Do not rewrite
    // to a host-language arithmetic expression.
    for (id, expression) in [
        ("34", "(2 + 3) * 8 + 2"),
        ("35", "-(-42)"),
        ("36", "(100 - 16) - 42"),
        ("37", "-7 * -6"),
        ("38", "(7 - 14) * -6"),
        ("39", "9223372036854775807 - 9223372036854775765"),
        ("40", "-9223372036854775808 + 9223372036854775807 + 43"),
        ("41", "2 * (3 + 4) * 3"),
        ("50", "2 * 3 * 7"),
        ("51", "(0 - 7) * (0 - 6)"),
        ("52", "(21 + 21) + (0 - 0)"),
        ("53", "6 * 7"),
        ("54", "(-7) * (-6)"),
        ("55", "9223372036854775807 - 9223372036854775807 + 42"),
        ("56", "-9223372036854775808 - (-9223372036854775808) + 42"),
        ("57", "(100 - 58) * (0 + 1)"),
    ] {
        let source = format!(
            "// {id}: candidate source-identical checked-i64 scalar program.\n\
             // Expected integer result 42; guest execution on each backend is unverified.\n\
             pub fnc main(): int {{ return {expression}; }}\n"
        );
        fixture(&source, &[], true, "42\n");
    }
    for expression in [
        "9223372036854775808",
        "7 / 2",
        "-(-9223372036854775808)",
        "5 ^ 3",
    ] {
        fixture(
            &format!("pub fnc main(): int {{ return {expression}; }}"),
            &[],
            false,
            "",
        );
    }
}

#[test]
fn signed_i64_boundary_identities_execute_in_native_cranelift_jit() {
    // These are actual Oreslang guest expressions. No Rust arithmetic oracle
    // or conversion through f64 may be substituted for the native result.
    for (expression, expected) in [
        ("0 - 0", "0\n"),
        ("-9223372036854775808 + 1", "-9223372036854775807\n"),
        ("9223372036854775807 - 1", "9223372036854775806\n"),
        ("(-9223372036854775807) - 1", "-9223372036854775808\n"),
        ("9223372036854775807 + 0", "9223372036854775807\n"),
        ("(0 - 42) * -1", "42\n"),
        ("-(-9223372036854775807)", "9223372036854775807\n"),
        ("(6 * 7) + (2 - 2)", "42\n"),
    ] {
        fixture(
            &format!("pub fnc main(): int {{ return {expression}; }}"),
            &[],
            true,
            expected,
        );
    }
    // Run-time parameters cannot be folded by the host language.
    fixture(
        "pub fnc main(x:int):int{return x+1;}",
        &["41"],
        true,
        "42\n",
    );
    fixture(
        "pub fnc main(x:int,y:int):int{return x*y;}",
        &["6", "7"],
        true,
        "42\n",
    );
}

#[test]
fn scalar_cast_and_ownership_features_fail_closed_before_jit_lowering() {
    // Java #2405, LLVM #1256, .NET #282 and Wasm #219 have staged
    // scalar-cast contracts; Cranelift now supports only *identity i64*
    // casts, not bool, Option, dynamic tags or actor ownership lowering.
    // These MUST be reclassified with native-positive receipts before
    // granting capability parity. A generic parser error is not parity.
    for source in [
        "pub fnc main(): int { return 42 as bool; }",
        "pub fnc main(): int { return 42 as Object; }",
        "pub fnc main(as:int):int{return as;}",
        "pub fnc main(): int { return 42 as? int; }",
        "pub fnc main(): int { const x = true; return 42; }",
        "pub fnc main(): int { const x = 42 as bool; return 42; }",
        "pub fnc main(): int { let x = 41; x = 42; return x; }",
        "pub fnc main(): int { return rt borrow 42; }",
        "pub fnc main(): int { return match 42 over on 42 { 42; } end; }",
        "pub async fnc main(): int { return 42; }",
    ] {
        fixture(source, &[], false, "");
    }
}

#[test]
fn orestra_capacity_arithmetic_is_checked_in_native_jit() {
    // Orestra reservation math depends on subtract-first i64 arithmetic.
    // These are scalar intermediates only: this backend does NOT run the
    // allocator, sealed actor state, a container, or any Orestra controller.
    for (expression, expected) in [
        ("(1000 - 600) - 358", "42\n"),
        ("(9223372036854775807 - 9223372036854775700) - 65", "42\n"),
        ("(9223372036854775807 - 9223372036854775765)", "42\n"),
        ("(-9223372036854775808 + 100) + 9223372036854775750", "42\n"),
        ("1000000 - 999958", "42\n"),
        ("(0 - 1) * -42", "42\n"),
        ("(9223372036854775807 - 9223372036854775806) * 42", "42\n"),
        ("(0 - 9223372036854775807) + 9223372036854775807", "0\n"),
    ] {
        fixture(
            &format!("pub fnc main(): int {{ return {expression}; }}"),
            &[],
            true,
            expected,
        );
    }
    // Checked overflow in an intermediate cannot be disguised by
    // multiplication by zero or subtraction of another huge value.
    for expression in [
        "9223372036854775807 - (-1)",
        "-9223372036854775808 - 1",
        "(9223372036854775807 + 1) * 0",
        "(9223372036854775807 * 2) - 9223372036854775807",
        "(-9223372036854775808 * -1)",
        "-(-9223372036854775808)",
    ] {
        fixture(
            &format!("pub fnc main(): int {{ return {expression}; }}"),
            &[],
            false,
            "",
        );
    }
    fixture(
        "pub fnc main(x:int):int{return x+1;}",
        &["9223372036854775807"],
        false,
        "",
    );
    fixture(
        "pub fnc main(x:int):int{return x-1;}",
        &["-9223372036854775808"],
        false,
        "",
    );
    fixture(
        "pub fnc main(x:int,y:int):int{return x*y;}",
        &["4611686018427387904", "2"],
        false,
        "",
    );
}

#[test]
fn rt_mut_share_is_never_lowered_to_scalar_clif() {
    // Every context must reject before machine-code emission. No ordinary
    // parser error or stderr-free JIT failure counts as a capability gate.
    const GATE: &str = "native rt mut share requires verified SharedMutex<T> capability";
    for (index, expression) in [
        "rt mut share 42",
        "(rt mut share 42)",
        "1 + (rt mut share 42)",
        "Integer.bitXor(rt mut share 1, 2)",
        "rt mut share (rt mut share 1)",
        "rt mut share (1 + 2)",
    ].iter().enumerate() {
        let file = std::env::temp_dir().join(format!(
            "orescranelift-mutshare-{}-{index}.ores", std::process::id()
        ));
        std::fs::write(&file, format!("fnc main():int{{return {expression};}}\n")).unwrap();
        for command in ["run", "ir"] {
            let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(command)
                .arg(&file)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(1), "{command}: unexpected compiler exit");
            assert!(result.stdout.is_empty(), "{command}: emitted executable value or CLIF IR");
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(stderr.contains(GATE), "{command}: missing ownership diagnostic: {stderr}");
            assert!(!stderr.contains("panic"), "{command}: compiler crashed");
        }
        std::fs::remove_file(file).unwrap();
    }
}



#[test]
fn ownership_rt_keyword_cannot_be_redeclared_as_scalar_parameter() {
    fixture("fnc main(rt:int):int{return rt;}", &["42"], false, "");
    fixture("fnc main(rt:int):int{return rt mut share 1;}", &["42"], false, "");
}


#[test]
fn rt_mut_share_inside_comments_does_not_change_scalar_execution() {
    fixture(
        "fnc main():int{return 42;} // rt mut share owned\n",
        &[], true, "42\n"
    );
    fixture(
        "fnc main():int{return 40+2;} // a second rt mut share\n",
        &[], true, "42\n"
    );
}


#[test]
fn five_backend_checked_i64_identity_cast_jit_execution() {
    // Native JIT guest source, not a Rust arithmetic model.
    for expression in [
        "(7 as int)+35",
        "(7 as int as int)+35",
        "((2+5) as int)*6",
    ] {
        fixture(&format!("pub fnc main():int{{return {expression};}}"), &[], true, "42\n");
    }
    for expression in [
        "7 as bool",
        "true as int",
        "9223372036854775808 as int",
        "7 as? int",
    ] {
        fixture(&format!("pub fnc main():int{{return {expression};}}"), &[], false, "");
    }
}


#[test]
fn unsupported_checked_cast_emits_no_clif_or_guest_output() {
    // A failed cast must not become an executable JIT artifact or an IR
    // disclosure. Test both CLI modes against an actual source file.
    const ERROR: &str = "unsupported checked cast target";
    for (index, expression) in [
        "7 as bool",
        "7 as string",
        "7 as Object",
        "7 as int as bool",
    ].iter().enumerate() {
        let nonce = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-cast-reject-{}-{nonce}-{index}.ores",
            std::process::id()
        ));
        std::fs::write(
            &path,
            format!("pub fnc main():int{{return {expression};}}\n"),
        )
        .unwrap();
        for mode in ["run", "ir"] {
            let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(1), "{mode}: invalid failure exit");
            assert!(result.stdout.is_empty(), "{mode}: emitted guest value/CLIF");
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(stderr.contains(ERROR), "{mode}: wrong diagnostic: {stderr}");
            assert!(!stderr.contains("panicked"), "{mode}: compiler panicked");
        }
        std::fs::remove_file(path).unwrap();
    }
}


// Same eight exact UTF-8 expressions pinned in
// oreslang-platform-conformance PR #275 (six values + two checked overflows).
// This executes *real* Cranelift JIT guest code when cargo tests actually run.
// Merely authoring these cases does NOT prove native execution or other backends.
#[test]
fn pinned_eight_case_checked_i64_parity_guest() {
    for (source, stdout) in [
        ("pub fnc main(): int { return 42; }\n", "42\n"),
        ("pub fnc main(): int { return 4 + 6 * (5 - 1); }\n", "28\n"),
        ("pub fnc main(): int { return -(7 * 3); }\n", "-21\n"),
        ("pub fnc main(): int { return -9223372036854775808; }\n", "-9223372036854775808\n"),
        ("pub fnc main(): int { return 9223372036854775807; }\n", "9223372036854775807\n"),
        ("pub fnc main(): int { return 9223372036854775807 - 1; }\n", "9223372036854775806\n"),
    ] {
        fixture(source, &[], true, stdout);
    }
    for source in [
        "pub fnc main(): int { return 9223372036854775807 + 1; }\n",
        "pub fnc main(): int { return 4611686018427387904 * 2; }\n",
    ] {
        fixture(source, &[], false, "");
    }
}

#[test]
fn checked_i64_parity_overflow_never_uses_wrapped_intermediates() {
    for (source, args) in [
        ("fnc main(x:int):int{return x+1;}", &["9223372036854775807"][..]),
        ("fnc main(x:int):int{return x-1;}", &["-9223372036854775808"][..]),
        ("fnc main(x:int):int{return x*2;}", &["4611686018427387904"][..]),
        ("fnc main(x:int):int{return (x+1)%-1;}", &["9223372036854775807"][..]),
    ] {
        fixture(source, args, false, "");
    }
}


#[test]
fn oreslana_portable_scalar_capacity_and_nonzero_guest_execution() {
    // Every call compiles actual Oreslang source to native Cranelift JIT
    // machine code. These do NOT test classes, Array<int>, actors or WAL.
    for (source, expected) in [
        ("fnc main(): int { return (4096 - 64) + (32 * 2) - 4096; }\n", "0\n"),
        ("pub fnc main(): int { return 73; }\n", "73\n"),
        ("pub fnc main(): int { return (9223372036854775807 - 9223372036854775800) * 6; }\n", "42\n"),
    ] {
        fixture(source, &[], true, expected);
    }
    // A late overflowing intermediate must not be rescued by a later zero.
    fixture(
        "pub fnc main(): int { return (9223372036854775807 + 1) * 0; }\n",
        &[], false, "",
    );
    // Fail closed on precisely the Oreslana structures unsupported by the
    // independent scalar-only Cranelift frontend.
    for source in [
        "define pub class OreslanaLedger as end",
        "pub fnc main():int{ let mut Array<int> accounts = new Array<int>(); return 0; }",
        "pub fnc main():int{ return Some(42).unwrap(); }",
    ] {
        fixture(source, &[], false, "");
    }
}

#[test]
fn byte_identical_v2_checked_cast_guests_execute_or_reject() {
    // These files must remain byte-identical to Java #2405, LLVM #1256,
    // Wasm #219 and .NET #282, not translated into Cranelift-specific source.
    // Git blob SHA-1 positive: 1a80e07da1289c87d04454866883f4e06cfc8d77
    // Git blob SHA-1 negative: ecc4ccea80f016243e38f351df60cfad9eb6e136
    let positive = include_str!("../contracts/v1/portable-checked-cast-positive-v2.ores");
    let negative = include_str!("../contracts/v1/portable-checked-cast-negative-v2.ores");
    assert_eq!(
        positive.as_bytes(),
        b"pub fnc main():int{return (7 as int)+(35 as int);}\n"
    );
    assert_eq!(
        negative.as_bytes(),
        b"pub fnc main():int{return (7 as int) as bool;}\n"
    );
    // Actual Oreslang source -> Cranelift JIT -> stdout, not a host oracle.
    fixture(positive, &[], true, "42\n");
    // Unsupported int-to-bool cast must not generate a successful guest.
    fixture(negative, &[], false, "");
}
