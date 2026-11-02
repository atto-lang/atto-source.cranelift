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
fn rate_limit_weighted_counter_scalar_native_execution() {
    // Genuine CLIF-JIT guest computations for the shared *scalar*
    // rate-limit arithmetic intersection. No quota authority is implemented.
    const SOURCE: &str = "fnc main(limit:int,used:int,units:int):int{return limit-used-units;}";
    for (args, expected) in [
        (["100", "60", "15"], "25\n"),
        (["100", "99", "1"], "0\n"),
        (["100", "99", "2"], "-1\n"),
        (["9007199254740991", "9007199254740990", "1"], "0\n"),
    ] {
        fixture(SOURCE, &args, true, expected);
    }
    fixture("fnc main():int{return 100-60-15;}", &[], true, "25\n");
    fixture(
        "fnc main():int{return 9007199254740991-9007199254740990;}",
        &[],
        true,
        "1\n",
    );
    // Signed64 overflow and unsupported features MUST reject before emitting
    // a value. This does not confer uint64 wire admission.
    fixture(SOURCE, &["9223372036854775807", "-1", "0"], false, "");
    fixture(SOURCE, &["-9223372036854775808", "1", "0"], false, "");
    fixture(
        "fnc main():int{return 9223372036854775807+1;}",
        &[],
        false,
        "",
    );
    fixture("fnc main():int{return 100/60;}", &[], false, "");
    fixture("fnc main():int{return 100>60;}", &[], false, "");
    fixture(
        "define module backpressure as pub fnc limit():int{return 42;} end",
        &[],
        false,
        "",
    );
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
    // The removed '%' source spelling must fail even inside named bitXor.
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
fn removed_modulo_operator_never_executes_native_code() {
    // Every historical positive remainder fixture is now a compile reject.
    // The backend must emit no stdout and must never JIT these programs.
    for (source, args) in [
        ("fnc main():int{return -7%3;}", &[][..]),
        ("fnc main():int{return 7%-3;}", &[][..]),
        ("fnc main():int{return -7%-3;}", &[][..]),
        ("fnc main():int{return 8%3*2;}", &[][..]),
        ("fnc main():int{return 20%6%3;}", &[][..]),
        ("fnc main():int{return -9223372036854775808%-1;}", &[][..]),
        (
            "fnc main(x:int):int{return x%-1;}",
            &["-9223372036854775808"][..],
        ),
        (
            "fnc main(x:int):int{return x%-9223372036854775808;}",
            &["7"][..],
        ),
        ("fnc main():int{return 8-7%3*2;}", &[][..]),
        ("fnc main(x:int):int{return (x+1)%-1;}", &["7"][..]),
        ("fnc main(x:int):int{return x%0;}", &["7"][..]),
        ("fnc main(x:int,y:int):int{return x%y;}", &["7", "2"][..]),
        ("fnc main(x:int):int{return x%(1+1);}", &["7"][..]),
        ("fnc main(x:int):int{return x%-x;}", &["7"][..]),
        (
            "fnc main(x:int):int{return x%-9223372036854775809;}",
            &["7"][..],
        ),
    ] {
        fixture(source, args, false, "");
    }
    // Prove unaffected named bit XOR executes actual Cranelift machine code.
    fixture(
        "fnc main():int{return Integer.bitXor(40,2);}",
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
    ]
    .iter()
    .enumerate()
    {
        let file = std::env::temp_dir().join(format!(
            "orescranelift-mutshare-{}-{index}.ores",
            std::process::id()
        ));
        std::fs::write(&file, format!("fnc main():int{{return {expression};}}\n")).unwrap();
        for command in ["run", "ir"] {
            let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(command)
                .arg(&file)
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(1),
                "{command}: unexpected compiler exit"
            );
            assert!(
                result.stdout.is_empty(),
                "{command}: emitted executable value or CLIF IR"
            );
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                stderr.contains(GATE),
                "{command}: missing ownership diagnostic: {stderr}"
            );
            assert!(!stderr.contains("panic"), "{command}: compiler crashed");
        }
        std::fs::remove_file(file).unwrap();
    }
}

#[test]
fn ownership_rt_keyword_cannot_be_redeclared_as_scalar_parameter() {
    fixture("fnc main(rt:int):int{return rt;}", &["42"], false, "");
    fixture(
        "fnc main(rt:int):int{return rt mut share 1;}",
        &["42"],
        false,
        "",
    );
}

#[test]
fn rt_mut_share_inside_comments_does_not_change_scalar_execution() {
    fixture(
        "fnc main():int{return 42;} // rt mut share owned\n",
        &[],
        true,
        "42\n",
    );
    fixture(
        "fnc main():int{return 40+2;} // a second rt mut share\n",
        &[],
        true,
        "42\n",
    );
}

#[test]
fn native_platform_portability_v1_checked_i64_source_matrix() {
    // Exact source text is shared with conformance PR #299, rather than
    // rewriting the arithmetic in a Rust host oracle. Compilation and the
    // printed i64 result must come from the actual native Cranelift JIT.
    // The source hashes and independent CI provenance still need verification
    // before cross-backend or platform certification.
    for (source, result) in [
        ("pub fnc main(): int { return 40 + 2; }\n", "42\n"),
        ("pub fnc main(): int { return (2 + 3) * 8; }\n", "40\n"),
        ("pub fnc main(): int { return 0 - 42; }\n", "-42\n"),
        ("pub fnc main(): int { return (30 - 10) - 10; }\n", "10\n"),
        ("pub fnc main(): int { return 6 * (10 - 3); }\n", "42\n"),
        ("pub fnc main(): int { return 7 - 7; }\n", "0\n"),
        ("pub fnc main(): int { return 21 + 21; }\n", "42\n"),
        ("pub fnc main(): int { return 7 * 6; }\n", "42\n"),
    ] {
        fixture(source, &[], true, result);
    }
    // An unsuccessful compiler check must not be mistaken for runtime
    // overflow or a successful guest execution; no stdout is permitted.
    fixture(
        "pub fnc main(): int { return missing_platform_binding; }\n",
        &[],
        false,
        "",
    );
    fixture(
        "pub fnc main(): int { return absent_platform_function; }\n",
        &[],
        false,
        "",
    );
}

#[test]
fn native_platform_portability_v1_rejects_fake_backend_features() {
    // A scalar-only Cranelift JIT must never silently accept Oreslang actor,
    // suspension, ownership or unrelated host-language constructs.
    for source in [
        "pub async fnc main(): int { return 42; }\n",
        "pub critical suspends fnc main(): int { return 42; }\n",
        "pub fnc main(): int { return await 42; }\n",
        "pub fnc main(): int { return unknown_runtime_platform_helper(42); }\n",
    ] {
        fixture(source, &[], false, "");
    }
}
