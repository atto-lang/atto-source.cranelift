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
        assert!(
            output.stderr.is_empty(),
            "successful native execution emitted stderr"
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    } else {
        // The scalar CLI's owned compiler rejection is exit(1) with a
        // structured diagnostic. A SIGABRT/panic/loader error is not a pass.
        assert_eq!(
            output.status.code(),
            Some(1),
            "negative fixture must reject through the native compiler"
        );
        assert!(
            output.stdout.is_empty(),
            "rejected compilation printed stdout"
        );
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.starts_with("orescranelift: "),
            "rejection requires a compiler-owned diagnostic: {diagnostic}"
        );
        assert!(
            !diagnostic.contains("panicked at") && !diagnostic.contains("thread 'main' panicked"),
            "compiler crash is not an admitted negative fixture"
        );
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
    fixture(
        "fnc main():int{return Integer.bitXor(7-3,Integer.bitXor(4,2));}",
        &[],
        true,
        "2\n",
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
fn removed_remainder_operator_is_rejected_before_guest_execution() {
    for source in [
        "fnc main():int{return -7%3;}",
        "fnc main():int{return 7%-3;}",
        "fnc main():int{return -7%-3;}",
        "fnc main():int{return 8%3*2;}",
        "fnc main():int{return 20%6%3;}",
        "fnc main():int{return -9223372036854775808%-1;}",
        "fnc main():int{return 8-7%3*2;}",
        "fnc main(x:int):int{return x%0;}",
        "fnc main(x:int,y:int):int{return x%y;}",
        "fnc main(x:int):int{return x%(1+1);}",
        "fnc main(x:int):int{return x%-x;}",
        "fnc main(x:int):int{return x%-9223372036854775809;}",
        "fnc main():int{return Integer.bitXor(7%3,1);}",
    ] {
        fixture(source, &[], false, "");
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
                String::from_utf8_lossy(&result.stderr).contains(if source == &link {
                    "source must be a regular file (symlinks forbidden)"
                } else {
                    "source must be a regular file"
                }),
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
        // Shared six-vector checked-denial intersection with LLVM->Wasm.
        "1 + (rt mut share 2)",
        "rt mut share (rt borrow 3)",
        "rt mut share (rt mut borrow 4)",
        "rt mut share (rt mut share 5)",
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
                stderr.starts_with("orescranelift: "),
                "{command}: no native compiler-owned diagnostic: {stderr}"
            );
            assert!(
                stderr.contains(GATE),
                "{command}: missing ownership diagnostic: {stderr}"
            );
            assert!(
                !stderr.contains("panicked at") && !stderr.contains("thread 'main' panicked"),
                "{command}: compiler crashed"
            );
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
fn injected_mutable_heap_state_denies_real_jit_before_output() {
    for source in [
        "@Inject(MutableStateContainer()) define class Counter as state { count: int = 0; } end",
        "@Inject(MutableStateContainer(scope = #singleton)) define singleton Counter as state {} end",
        "fnc main():int{return 1;} @Inject(MutableStateContainer())",
    ] {
        fixture(source, &[], false, "");
    }
    // A comment never changes the native scalar language's semantics.
    fixture(
        "// @Inject(MutableStateContainer()) is only documentation\nfnc main():int{return 42;}",
        &[],
        true,
        "42\n",
    );
}

#[test]
fn injected_state_diagnostic_is_specific_and_no_ir_is_emitted() {
    let source = "@Inject(MutableStateContainer()) define class Counter as state {} end";
    let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "orescranelift-injection-denial-{}-{suffix}.ores",
        std::process::id()
    ));
    std::fs::write(&file, source).unwrap();
    for mode in ["run", "ir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{mode} unexpectedly accepted state injection"
        );
        assert!(output.stdout.is_empty(), "{mode} emitted native output");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("STATE_INJECTION_UNSUPPORTED: @Inject"),
            "{diagnostic}"
        );
        assert!(
            diagnostic
                .contains("requires owned state, typed borrow checking and lifecycle lowering"),
            "{diagnostic}"
        );
    }
    std::fs::remove_file(file).unwrap();
}

#[test]
fn flags2env_client_abi_status_scalar_native_execution() {
    const SOURCE: &str = include_str!("fixtures/flags2env-client-abi-status.ores");
    const EXPECTED: &str = "// Shared scalar ABI smoke for every Oreslang backend, including Cranelift.\n// Encodes public host statuses 0..4 without imports, strings or control flow.\npub fnc main(): int {\n  return (0 * 10000) + (1 * 1000) + (2 * 100) + (3 * 10) + 4 - 1192;\n}\n";
    assert_eq!(
        SOURCE.as_bytes(),
        EXPECTED.as_bytes(),
        "flags2env fixture bytes drifted"
    );
    fixture(SOURCE, &[], true, "42\n");
}

#[test]
fn heap_container_source_refuses_scalar_only_jit_fallback() {
    // Execute the actual scalar CLI, rather than matching text in a contract.
    // No Box, Arc, Weak, Vector, Array, borrowed heap or guarded mutation
    // lowering exists in this stage-0 Cranelift frontend. Fail closed.
    for (name, source) in [
        (
            "unique heap Box",
            "pub fnc main(): int { val Box<int> owner = Box<int>(42); return 42; }",
        ),
        (
            "atomic Arc",
            "pub fnc main(): int { val Arc<int> owner = Arc<int>(42); return 42; }",
        ),
        (
            "weak shared owner",
            "pub fnc main(): int { val Weak<int> weak = null; return 42; }",
        ),
        (
            "resizable Vector",
            "pub fnc main(): int { val Vector[int] values = [42]; return 42; }",
        ),
        (
            "resizable Array",
            "pub fnc main(): int { val Array<int> a = new Array<int>(); a.add(42); return a.get(0); }",
        ),
        (
            "unique move after take",
            "pub fnc main(): int { let int value = 42; let moved = rt take value; return value; }",
        ),
        (
            "borrow heap value",
            "pub fnc main(): int { let int value = 42; let loan = rt borrow value; return loan; }",
        ),
        (
            "exclusive heap borrow",
            "pub fnc main(): int { let int value = 42; let loan = rt mut borrow value; return loan; }",
        ),
        (
            "mut share without synchronized proof",
            "pub fnc main(): int { let int value = 42; rt mut share value; return value; }",
        ),
        (
            "shared guard state",
            "pub fnc main(): int { val Mutex<int> cell = Mutex<int>(42); return 42; }",
        ),
    ] {
        assert!(!source.is_empty(), "{name} fixture must be nonempty");
        fixture(source, &[], false, "");
    }
    // Keep the already supported common scalar intersection executable.
    fixture("pub fnc main(): int { return 21 + 21; }", &[], true, "42\n");
}

#[test]
fn source_testkit_async_callback_task_and_actor_syntax_fails_closed() {
    // Scalar-only Cranelift must not silently accept a test body that creates
    // async work it cannot join, observe, cancel or physically retire.
    // These are capability *rejection* checks, NOT successful TestKit parity.
    for source in [
        "pub async fnc main(): int { return 42; }",
        "suspends fnc main(): int { return 42; }",
        "fnc main():int{return await 42;}",
        "fnc main():int{return (async task || -> {return 42;})();}",
        "fnc main():int{return (|int x| -> x)(42);}",
        "fnc main():int{return spawn Worker();}",
        "define actor Worker as end\nfnc main():int{return 42;}",
        "import fnc run_suite from 'ores.std/testing';\nfnc main():int{return 42;}",
        "fnc main():int{return trap worker.send(42);}",
        "fnc main():int{return 42;}\nfnc test_async():int{return 7;}",
    ] {
        fixture(source, &[], false, "");
    }
    // Reject only *syntax*, not words embedded in comments.
    fixture(
        "fnc main():int{return 42;} // async task callback actor await",
        &[],
        true,
        "42\n",
    );
}

#[test]
fn orie_portable_checked_scalar_native_jit() {
    // Exact Oreslang source bytes from the ORIE portable scalar candidate.
    // This test invokes the *real* Cranelift compiler/JIT CLI, not a Rust
    // arithmetic oracle. An authored fixture is not a CI execution receipt.
    let entry = include_str!("fixtures/orie/entry_42.ores");
    fixture(entry, &[], true, "42\n");

    let overflow = include_str!("fixtures/orie/checked_overflow.ores");
    let nonce = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-orie-portable-{}-{nonce}.ores",
        std::process::id()
    ));
    std::fs::write(&path, overflow).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        result.status.code(),
        Some(1),
        "overflow must reject with status 1"
    );
    assert!(result.stdout.is_empty(), "overflow emitted a normal result");
    let diagnostic = String::from_utf8_lossy(&result.stderr);
    assert!(
        diagnostic.contains("i64 arithmetic overflow"),
        "expected a checked-overflow diagnosis, got: {diagnostic}"
    );
}

#[test]
fn reserved_actor_control_and_ownership_parameters_never_execute() {
    // This stage-0 JIT cannot implement actor supervision or ownership guards.
    // Reserved operation words must never be accepted as scalar aliases.
    for keyword in [
        "rt", "panic", "raise", "throw", "recover", "defer", "spawn", "stop", "select", "match",
        "switch", "suspend", "share", "borrow", "take", "copy", "gpu", "cpu", "isoactor", "let",
        "val", "const", "final",
    ] {
        let source = format!("fnc main({keyword}:int):int{{return {keyword};}}");
        fixture(&source, &["42"], false, "");
    }
}

#[test]
fn snake_case_parameter_names_containing_reserved_words_still_execute() {
    for name in ["rt_count", "panic_count", "spawn_count", "gpu_cycles"] {
        let source = format!("fnc main({name}:int):int{{return {name};}}");
        fixture(&source, &["42"], true, "42\n");
    }
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
        (
            "fnc main(x:int):int{return x+1;}",
            &["9223372036854775807"][..],
        ),
        (
            "fnc main(x:int):int{return x-1;}",
            &["-9223372036854775808"][..],
        ),
        (
            "fnc main(x:int):int{return -x;}",
            &["-9223372036854775808"][..],
        ),
        (
            "fnc main(x:int):int{return x*2;}",
            &["4611686018427387904"][..],
        ),
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
    fixture("fnc main(x:int):int{return x;}", &["not-an-int"], false, "");
    fixture("fnc main(x:int):int{return x;}", &["42", "43"], false, "");
    fixture("fnc main(x:int):int{return x;}", &["42"], true, "42\n");
}

#[test]
fn native_four_parameter_order_is_stable() {
    fixture(
        "fnc main(a:int,b:int,c:int,d:int):int{return a*1000+b*100+c*10+d;}",
        &["1", "2", "3", "4"],
        true,
        "1234\n",
    );
    fixture(
        "fnc main(a:int,b:int,c:int,d:int):int{return a-b-c-d;}",
        &["100", "20", "10", "5"],
        true,
        "65\n",
    );
    fixture(
        "fnc main(a:int,b:int,c:int,d:int):int{return a+b+c+d;}",
        &["1", "2", "3", "4"],
        true,
        "10\n",
    );
}

#[test]
fn native_signed_parameter_extremes_preserve_full_i64() {
    for (arg, expected) in [
        ("-9223372036854775808", "-9223372036854775808\n"),
        ("9223372036854775807", "9223372036854775807\n"),
        ("-9007199254740993", "-9007199254740993\n"),
        ("9007199254740993", "9007199254740993\n"),
        ("0", "0\n"),
        ("-1", "-1\n"),
    ] {
        fixture("fnc main(x:int):int{return x;}", &[arg], true, expected);
    }
}

#[test]
fn malformed_numeric_digit_separators_reject_without_stdout() {
    for expression in [
        "1_",
        "1__0",
        "9_223__372",
        "99___99",
        "-1__0",
        "-9_223_372_036__854_775_808",
        "-9223372036854775808_",
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
fn checked_scalar_arithmetic_rejects_all_overflow_signs() {
    for (source, args) in [
        (
            "fnc main(x:int):int{return x+1;}",
            &["9223372036854775807"][..],
        ),
        (
            "fnc main(x:int):int{return x-1;}",
            &["-9223372036854775808"][..],
        ),
        (
            "fnc main(x:int):int{return -x;}",
            &["-9223372036854775808"][..],
        ),
        ("fnc main(x:int):int{return x*x;}", &["3037000500"][..]),
        (
            "fnc main(x:int):int{return 0-x;}",
            &["-9223372036854775808"][..],
        ),
    ] {
        fixture(source, args, false, "");
    }
}

#[test]
fn native_xor_intrinsic_handles_repeated_and_extreme_values() {
    for (expression, expected) in [
        ("Integer.bitXor(0,0)", "0\n"),
        ("Integer.bitXor(-1,-1)", "0\n"),
        ("Integer.bitXor(7,7)", "0\n"),
        ("Integer.bitXor(7,0)", "7\n"),
        ("Integer.bitXor(0,7)", "7\n"),
        (
            "Integer.bitXor(9223372036854775807,-1)",
            "-9223372036854775808\n",
        ),
        ("Integer.bitXor(5,Integer.bitXor(3,6))", "0\n"),
    ] {
        fixture(
            &format!("fnc main():int{{return {expression};}}"),
            &[],
            true,
            expected,
        );
    }
}

#[test]
fn commented_out_unsupported_tokens_are_not_guest_source() {
    fixture(
        "// unsafe actor invalid % ^ \nfnc main():int{return 42;}",
        &[],
        true,
        "42\n",
    );
    fixture(
        "fnc main():int{return 42;} // ^ never parsed",
        &[],
        true,
        "42\n",
    );
    fixture(
        "fnc main():int{return 1;} // ignored \nunsafe",
        &[],
        false,
        "",
    );
}

#[test]
fn oversized_and_deeply_nested_sources_fail_closed() {
    fixture(&" ".repeat(1024 * 1024 + 1), &[], false, "");
    let deep = format!(
        "fnc main():int{{return {}1{};}}",
        "(".repeat(140),
        ")".repeat(140)
    );
    fixture(&deep, &[], false, "");
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

#[test]
fn actor_finalization_v1_is_fail_closed_until_native_actor_lowering_exists() {
    // AF01–AF20: negative *admission* coverage only, never guest parity.
    // Cranelift currently compiles a scalar i64 subset and must never turn
    // actor lifecycle, ETS or supervisor CONTROL source into numeric output.
    for source in [
        "define actor Worker as end fnc main():int{return 42;}",
        "define serial actor Worker as end fnc main():int{return 42;}",
        "fnc main():int{spawn Worker();return 42;}",
        "fnc main():int{return spawn Worker();}",
        "fnc main():int{return actor.request_close();}",
        "fnc main():int{return supervisor.finalize_stop(1);}",
        "fnc main():int{return actor.restart();}",
        "fnc main():int{return table.give_away(1,2);}",
        "fnc main():int{return actor.handle_state(critical-read);}",
        "fnc main():int{return rt mut share 42;}",
        "fnc main():int{return panic(42);}",
        "fnc main():int{return raise(42);}",
        "fnc main():int{return await spawn Worker();}",
        "fnc main():int{return ets.insert_new(1,2);}",
    ] {
        fixture(source, &[], false, "");
    }
}

#[test]
fn actor_finalization_words_inside_comments_do_not_change_scalar_result() {
    // A safety gate must not reject unrelated source merely because a
    // comment mentions unsupported runtime operations.
    fixture(
        "fnc main():int{return 40+2;} // actor request_close panic supervisor CONTROL\n",
        &[],
        true,
        "42\n",
    );
    fixture(
        "fnc main():int{return Integer.bitXor(5,3);} // ETS give_away\n",
        &[],
        true,
        "6\n",
    );
}

#[test]
fn unsupported_suspension_and_critical_syntax_fail_closed_before_jit() {
    // The Cranelift frontend currently supports the scalar subset only.
    // These negative native guest tests must not be mistaken for actual
    // suspending continuation or critical-section runtime support.
    for source in [
        "suspends fnc main():int{return 1;}",
        "async fnc main():int{return 1;}",
        "critical fnc main():int{return 1;}",
        "critical suspends fnc main():int{return 1;}",
        "suspends critical fnc main():int{return 1;}",
        "fnc main():int{return await 1;}",
        "fnc main():int{critical {return 1;}}",
        "fnc main():int{try critical {return 1;} else {return 0;}}",
    ] {
        fixture(source, &[], false, "");
    }
    // Reject keywords as syntax, not as a textual substring in comments.
    fixture(
        "fnc main():int{return 7;} // suspends critical await",
        &[],
        true,
        "7\n",
    );
}

#[test]
fn tcp_connection_pool_and_actor_ownership_fail_closed_before_native_jit() {
    // Stage-0 Cranelift compiles scalar expressions only. These are not
    // silently desugared into a blocking socket, raw pointer or shared heap.
    // A rejected CLI must not write successful guest output to stdout.
    for source in [
        "define actor TcpPool as end fnc main():int{return 0;}",
        "fnc main():int{return TcpConnection.open();}",
        "fnc main():int{return TcpPool.acquire();}",
        "fnc main():int{return rt share socket;}",
        "fnc main():int{return rt mut share socket;}",
        "fnc main():int{return rt borrow connection;}",
        "fnc main():int{return rt ref socket;}",
        "fnc main():int{return rt deref socket;}",
        "fnc main():int{return Pointer<int>.from_raw(42);}",
        "fnc main():int{return await pool.acquire();}",
        "fnc main():int{return spawn TcpOwner();}",
        "fnc main():int{return socket.send(42);}",
        "fnc main():int{return unsafe;}",
        "fnc main():int{return 42;} pool",
    ] {
        fixture(source, &[], false, "");
    }
    // Only lexical comments mention the future TCP capability: the
    // actual compiled guest remains a source-only scalar expression.
    fixture(
        "fnc main():int{return 42;} // tcp pool rt share",
        &[],
        true,
        "42\n",
    );
}

#[test]
fn heap_ownership_requires_real_lowering_in_run_and_ir() {
    // A scalar-only JIT must not silently produce native machine code for
    // references into relocatable Array<T> or shared heap handles. A parser
    // failure is a supported-feature denial, NOT a borrow-safety proof.
    let unsupported = [
        "fnc main():int{return Array<int>.new();}",
        "fnc main():int{return Box<int>.new(42);}",
        "fnc main():int{return Arc<int>.new(42);}",
        "fnc main():int{return Rc<int>.new(42);}",
        "fnc main():int{return SharedMutex.new(42);}",
        "fnc main():int{return values[0];}",
        "fnc main():int{return values.add(42);}",
        "fnc main():int{return rt borrow values[0];}",
        "fnc main():int{return rt mut borrow values;}",
        "fnc main():int{return rt take values[0];}",
        "fnc main():int{return rt share shared_handle;}",
        "fnc main():int{return rt mut share shared_handle;}",
        "fnc main():int{return state.handle_state(42);}",
        "define class HeapBox as pub let int value=42; end fnc main():int{return 42;}",
    ];
    for (index, source) in unsupported.iter().enumerate() {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-heap-admission-{}-{}-{}.ores",
            std::process::id(),
            sequence,
            index
        ));
        std::fs::write(&path, source).unwrap();
        for mode in ["run", "ir"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(1),
                "{mode} admitted unsupported heap syntax or crashed in case {index}"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} emitted guest output/CLIF for unsupported heap case {index}"
            );
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                diagnostic.starts_with("orescranelift: "),
                "{mode} lacked a frontend diagnostic for case {index}: {diagnostic}"
            );
            assert!(
                !diagnostic.contains("panicked at")
                    && !diagnostic.contains("thread 'main' panicked"),
                "{mode} crashed instead of rejecting heap case {index}"
            );
        }
        std::fs::remove_file(&path).unwrap();
    }
    fixture(
        "fnc main():int{return 42;} // Arc<int> Array<int> rt borrow",
        &[],
        true,
        "42\n",
    );
}

#[test]
fn readonly_shared_slab_lease_and_pin_fail_closed_in_run_and_ir() {
    // This backend has no trusted shared slab region, reactor, actor identity
    // authority, physical close attestation, or capability lease lowering.
    // Reject both execution AND emitted native IR to prevent false parity.
    let cases = [
        "fnc main():int{return tcp.pool(3);}",
        "fnc main():int{return pool.acquire_async();}",
        "fnc main():int{return lease.exchange_async(3);}",
        "fnc main():int{return lease.release_async();}",
        "fnc main():int{return FrozenBytes.from([1,2,3]);}",
        "fnc main():int{return FrozenBytes.pin();}",
        "fnc main():int{return FrozenBytes.read_at(0);}",
        "fnc main():int{return FrozenTcpSlab.release_pin(0);}",
        "fnc main():int{return FrozenTcpSlab.retire();}",
        "fnc main():int{return slab.read_byte(0);}",
        "fnc main():int{return slab.pin_count();}",
        "fnc main():int{return pool.try_confirm_physical_close(0);}",
        "fnc main():int{return SharedBuffer.pin();}",
        "fnc main():int{return shared_buffer.read_at(0);}",
        "fnc main():int{return rt share frozen_request;}",
        "fnc main():int{return rt mut share frozen_request;}",
        "fnc main():int{return await pool.open_stream_async();}",
        "define actor PoolOwner as end fnc main():int{return 1;}",
        "fnc main():int{return Pointer<int>.from_raw(1);}",
    ];
    for (index, source) in cases.iter().enumerate() {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-slab-{}-{}-{}.ores",
            std::process::id(),
            sequence,
            index
        ));
        std::fs::write(&path, source).unwrap();
        for mode in ["run", "ir"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .unwrap();
            // A signal, missing launcher, or infrastructure exit code is not
            // evidence that the native frontend rejected the feature.
            assert_eq!(
                output.status.code(),
                Some(1),
                "{mode} must return the compiler's explicit rejection status for case {index}"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} emitted guest output/IR for unsupported source case {index}"
            );
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                diagnostic.starts_with("orescranelift: "),
                "{mode} must emit the native compiler's owned diagnostic for case {index}: {diagnostic}"
            );
            assert!(
                !diagnostic.contains("panicked at")
                    && !diagnostic.contains("thread 'main' panicked"),
                "{mode} crashed instead of rejecting unsupported source case {index}"
            );
        }
        std::fs::remove_file(&path).unwrap();
    }
    fixture(
        "fnc main():int{return 17;} // shared readonly tcp slab pin",
        &[],
        true,
        "17\n",
    );
}

#[test]
fn actor_io_never_emits_scalar_value_or_clif_before_actor_ir_exists() {
    const IO_GATE: &str = "actor-local io.* requires native actor IR/ABI";
    const STDIO_GATE: &str = "process stdio is outside the Cranelift stage-0 scalar profile";

    for (index, (source, gate)) in [
        ("fnc main():int{return io;}", IO_GATE),
        ("fnc main():int{return io.out;}", IO_GATE),
        ("fnc main():int{return stdio;}", STDIO_GATE),
    ]
    .iter()
    .enumerate()
    {
        let file = std::env::temp_dir().join(format!(
            "orescranelift-actor-io-{}-{index}.ores",
            std::process::id()
        ));
        std::fs::write(&file, source).unwrap();
        for command in ["run", "ir"] {
            let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(command)
                .arg(&file)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(1), "{command}: unexpected exit");
            assert!(result.stdout.is_empty(), "{command}: emitted value or CLIF");
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                stderr.contains(gate),
                "{command}: missing actor-I/O gate: {stderr}"
            );
            assert!(!stderr.contains("panic"), "{command}: compiler crashed");
        }
        std::fs::remove_file(file).unwrap();
    }
}

#[test]
fn model_graph_source_cannot_reach_cranelift_native_jit() {
    // These are actual CLI/compiled-guest admission paths, not a host
    // string-matching oracle. No IR or result may be published.
    for source in [
        "define model x as Identity() end",
        "define model PriceEstimate() as Identity() end",
        "define pub model x as Identity() end",
        "pub define model x as Identity() end",
    ] {
        fixture(source, &[], false, "");
    }
    // The contextual identifier remains valid in the scalar profile.
    fixture(
        "fnc main(model:int):int{return model+1;}",
        &["41"],
        true,
        "42\n",
    );
}

#[test]
fn native_java_v1_reserved_parameter_gate_and_scalar_controls() {
    // These are actual CLI compile/JIT boundaries, not a synthetic lexer dump.
    // The scalar backend deliberately rejects language forms it cannot lower.
    for word in [
        "eq", "neq", "is", "isoactor", "null", "panic", "cpu", "gpu", "recover", "throw", "raise",
        "defer", "nb", "select",
    ] {
        fixture(
            &format!("fnc main({word}:int):int{{return {word};}}"),
            &["42"],
            false,
            "",
        );
    }
    for name in ["owned", "eqx", "xneq", "island", "nullish", "panic_mode"] {
        fixture(
            &format!("fnc main({name}:int):int{{return {name};}}"),
            &["42"],
            true,
            "42\n",
        );
    }
    for expression in [
        "#ready",
        "nb-select",
        "critical-read",
        "transform-pipe",
        "gpu",
        "cpu",
        "panic",
        "1 ^ 2",
    ] {
        fixture(
            &format!("fnc main():int{{return {expression};}}"),
            &[],
            false,
            "",
        );
    }
    fixture("fnc main():int{return 7%3;}", &[], false, "");
}

#[test]
fn odb1_native_signed_shift8_scalar_gate() {
    // Real native JIT guest checks of the ODB1 scalar byte extraction floor.
    // DOES NOT establish WAL List/Result/actor/module support.
    let source = "fnc main(x:int):int{return x>>8;}";
    for (operand, expected) in [
        ("0", "0\n"),
        ("255", "0\n"),
        ("256", "1\n"),
        ("65535", "255\n"),
        ("2147483647", "8388607\n"),
        ("-1", "-1\n"),
        ("-256", "-1\n"),
        ("-9223372036854775808", "-36028797018963968\n"),
    ] {
        fixture(source, &[operand], true, expected);
    }
    fixture("fnc main(x:int):int{return x+1>>8;}", &["255"], true, "1\n");
    fixture(
        "fnc main(x:int):int{return x+1>>8;}",
        &["9223372036854775807"],
        false,
        "",
    );
    fixture(
        "fnc main(x:int):int{return x>>8>>8;}",
        &["65536"],
        true,
        "1\n",
    );
    // No silent lowering of any other shift, dynamic count, float division,
    // unsigned shift or module/ADT heap features.
    for invalid in [
        "fnc main(x:int):int{return x>>7;}",
        "fnc main(x:int,y:int):int{return x>>y;}",
        "fnc main(x:int):int{return x>>8+1;}",
        "fnc main(x:int):int{return x>>>8;}",
        "fnc main(x:int):int{return x>8;}",
        "fnc main(x:int):int{return x<<8;}",
        "fnc main(x:int):int{return x/256;}",
        "define module wal_frame as pub fnc main():int{return 256>>8;} end",
    ] {
        fixture(invalid, &["256"], false, "");
    }
}

#[test]
fn actor_declarations_fail_closed_before_cranelift_ir_or_native_jit() {
    // These are real CLI tests of the shipping stage-0 frontend in both
    // execution and IR modes. They do not claim source actor execution.
    const DIAGNOSTIC: &str =
        "Cranelift source actor hierarchy, traits and mailbox runtime unsupported";
    for source in [
        "define actor A1 as end",
        "define pub actor A1 as end",
        "pub define actor A1 as end",
        "define isoactor A1 as end",
        "define untrusted actor A1 as end",
        "define serial http actor A1 as end",
        "define abstract actor A1 as end",
        "define transform actor A1 as end",
        "define pipe actor A1 as end",
        "shared actor A1 { }",
        "untrusted actor A1 { }",
        "isoactor A1 { }",
        "actor A1 as end",
        "actor worker as end",
        "actor fnc work():int{return 1;}",
        "fnc actor work():int{return 1;}",
        "define actor A1 extends Base as end",
        "define actor A1 implements Traced as end",
        "define actor A1 impl Traced as end",
        // Exactly the portable lifecycle v1 source, pinned in conformance.
        include_str!("fixtures/actor-execution-v1.ores"),
    ] {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-actor-gate-{}-{sequence}.ores",
            std::process::id()
        ));
        std::fs::write(&path, source).unwrap();
        for mode in ["run", "ir"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{mode}: {source}");
            assert!(
                output.stdout.is_empty(),
                "{mode} emitted guest value or CLIF IR: {source}"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(
                    if source.trim_start().starts_with("// Lifecycle")
                        || source.trim_start().starts_with("actor ")
                        || source.starts_with("define actor ")
                        || source.starts_with("define pub actor ")
                        || source.starts_with("pub define actor ")
                    {
                        "DECL_KIND_UNSUPPORTED: actor"
                    } else {
                        DIAGNOSTIC
                    }
                ),
                "{mode}: wrong actor failure for {source}: {stderr}"
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    fixture(
        "fnc main():int{return 42;} // define actor A1 as end",
        &[],
        true,
        "42\n",
    );
}

#[test]
fn unsupported_websocket_and_transport_actor_source_fails_before_native_jit() {
    // This is an intentional negative parity contract: the independent
    // stage-0 Cranelift backend has no actor isolation or network imports.
    // Do not report a successful scalar JIT as a working WebSocket guest.
    const DIAGNOSTIC: &str = "does not support network actor execution";
    for (index, source) in [
        "define actor Server as end",
        "define pub actor Server as end",
        "pub define actor Server as end",
        "define http actor Server as end",
        "define websocket actor Server as end",
        "define liveview actor Server as end",
        "define tcp actor Server as end",
        "define ws actor Server as end",
    ]
    .iter()
    .enumerate()
    {
        let nonce = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let file = std::env::temp_dir().join(format!(
            "orescranelift-transport-deny-{}-{nonce}-{index}.ores",
            std::process::id()
        ));
        std::fs::write(&file, source).unwrap();
        for mode in ["run", "ir"] {
            let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&file)
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(1),
                "{mode}: unsupported transport must fail before JIT"
            );
            assert!(
                result.stdout.is_empty(),
                "{mode}: rejected transport produced stdout"
            );
            assert!(
                String::from_utf8_lossy(&result.stderr).contains(
                    if source.contains("liveview") || source.contains(" ws ") {
                        DIAGNOSTIC
                    } else if source.contains(" http ")
                        || source.contains(" websocket ")
                        || source.contains(" tcp ")
                    {
                        "Cranelift source actor hierarchy, traits and mailbox runtime unsupported"
                    } else {
                        "DECL_KIND_UNSUPPORTED: actor"
                    }
                ),
                "{mode}: wrong diagnostic: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        std::fs::remove_file(file).unwrap();
    }
    // The same words in ordinary comments must not change scalar parsing.
    fixture(
        "fnc main():int{return 42;} // define websocket actor",
        &[],
        true,
        "42\n",
    );
    // A call-shaped guest API is unsupported as well, not lowered to native.
    fixture(
        "fnc main():int{return websocket.listen(1234);}",
        &[],
        false,
        "",
    );
}

#[test]
fn actor_supervisor_authority_fails_before_ir_or_execution() {
    // Neither host-native execution nor CLIF emission is allowed to interpret
    // actor authority without a real guest actor runtime and capability model.
    for source in [
        "define actor Worker as end",
        "define supervisor actor Manager as end",
        "define supervisor S1 as on_panic(f: Failure): void { return; } end",
        "pub define supervisor S1 as end",
        "define supervisor S1 extends Actor as end",
        "define supervisor S1 extends Supervisor as end",
        "define supervisor S1 implements Observer as end",
        "define supervisor S1 with RestartPolicy as end",
        "define proxy actor Remote as end",
        "fnc main():int{return spawn Worker();}",
        "fnc main():int{return stop;}",
        "fnc main(ActorRef:int):int{return 42;}",
        "fnc main(ActorId:int):int{return 42;}",
        "fnc main(Supervisor:int):int{return 42;}",
        "fnc main(SupervisorId:int):int{return 42;}",
        "fnc main(SupervisorRef:int):int{return 42;}",
        "fnc main():int{return 42;} actor",
    ] {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-authority-{}-{sequence}.ores",
            std::process::id()
        ));
        std::fs::write(&path, source).unwrap();
        for mode in ["run", "ir"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "{mode} unexpectedly admitted {source}"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} emitted native output or CLIF"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(
                    if source.starts_with("define actor") {
                        "DECL_KIND_UNSUPPORTED: actor"
                    } else if source.starts_with("define supervisor")
                        || source.starts_with("pub define supervisor")
                    {
                        "DECL_KIND_UNSUPPORTED: supervisor"
                    } else {
                        "actor and supervisor capabilities are unsupported in Cranelift stage-0"
                    }
                ),
                "{mode} did not produce capability-specific denial: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::remove_file(path).unwrap();
    }
    // Reserved capability words are whole tokens: ordinary prefixed names
    // remain scalar values and do not create supervisor authority.
    fixture(
        "fnc main(supervisor_count:int):int{return supervisor_count;}",
        &["42"],
        true,
        "42\n",
    );
    // Authority words in comments are not executable syntax.
    fixture(
        "fnc main():int{return 42;} // actor supervisor spawn stop ActorRef",
        &[],
        true,
        "42\n",
    );
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
fn five_backend_checked_i64_identity_cast_jit_execution() {
    // Native JIT guest source, not a Rust arithmetic model.
    for expression in ["(7 as int)+35", "(7 as int as int)+35", "((2+5) as int)*6"] {
        fixture(
            &format!("pub fnc main():int{{return {expression};}}"),
            &[],
            true,
            "42\n",
        );
    }
    for expression in [
        "7 as bool",
        "true as int",
        "9223372036854775808 as int",
        "7 as? int",
    ] {
        fixture(
            &format!("pub fnc main():int{{return {expression};}}"),
            &[],
            false,
            "",
        );
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
    ]
    .iter()
    .enumerate()
    {
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
            assert_eq!(
                result.status.code(),
                Some(1),
                "{mode}: invalid failure exit"
            );
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
        (
            "pub fnc main(): int { return -9223372036854775808; }\n",
            "-9223372036854775808\n",
        ),
        (
            "pub fnc main(): int { return 9223372036854775807; }\n",
            "9223372036854775807\n",
        ),
        (
            "pub fnc main(): int { return 9223372036854775807 - 1; }\n",
            "9223372036854775806\n",
        ),
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
        (
            "fnc main(x:int):int{return x+1;}",
            &["9223372036854775807"][..],
        ),
        (
            "fnc main(x:int):int{return x-1;}",
            &["-9223372036854775808"][..],
        ),
        (
            "fnc main(x:int):int{return x*2;}",
            &["4611686018427387904"][..],
        ),
        (
            "fnc main(x:int):int{return (x+1)*0;}",
            &["9223372036854775807"][..],
        ),
    ] {
        fixture(source, args, false, "");
    }
}

#[test]
fn orestra_scalar_capacity_and_execution_canaries_native_jit() {
    // Real Cranelift JIT source-to-machine-code tests, *not* orchestration.
    // Actors, scheduler reservations, imports and resource fencing remain
    // unavailable in this stage-0 scalar frontend.
    for (expression, expected) in [
        ("(1000 - 600) - 400", "0\n"),
        ("(9223372036854775807 - 9223372036854775700) - 65", "42\n"),
        ("73", "73\n"),
    ] {
        fixture(
            &format!("pub fnc main(): int {{ return {expression}; }}\n"),
            &[],
            true,
            expected,
        );
    }
    // An intermediate signed64 overflow must never wrap into a result.
    fixture(
        "pub fnc main(): int { return (9223372036854775807 + 1) - 1; }\n",
        &[],
        false,
        "",
    );
}

#[test]
fn oreslana_portable_scalar_capacity_and_nonzero_guest_execution() {
    // Every call compiles actual Oreslang source to native Cranelift JIT
    // machine code. These do NOT test classes, Array<int>, actors or WAL.
    for (source, expected) in [
        (
            "fnc main(): int { return (4096 - 64) + (32 * 2) - 4096; }\n",
            "0\n",
        ),
        ("pub fnc main(): int { return 73; }\n", "73\n"),
        (
            "pub fnc main(): int { return (9223372036854775807 - 9223372036854775800) * 6; }\n",
            "42\n",
        ),
    ] {
        fixture(source, &[], true, expected);
    }
    // A late overflowing intermediate must not be rescued by a later zero.
    fixture(
        "pub fnc main(): int { return (9223372036854775807 + 1) * 0; }\n",
        &[],
        false,
        "",
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

#[test]
fn native_scalar_and_unsupported_actor_guest_parity() {
    // Each fixture is actual native Oreslang source, compiled and executed by
    // the Cranelift JIT through its real CLI. Rust is only the test launcher.
    for (source, expected) in [
        (include_str!("parity/checked_precedence.ores"), "28\n"),
        (
            include_str!("parity/checked_i64_min.ores"),
            "-9223372036854775808\n",
        ),
        (
            include_str!("parity/checked_i64_max_minus_one.ores"),
            "9223372036854775806\n",
        ),
        (
            include_str!("parity/bitxor_named_intrinsic.ores"),
            "-9223372036854775808\n",
        ),
    ] {
        fixture(source, &[], true, expected);
    }

    // The scalar JIT is NOT allowed to silently accept or reinterpret actor,
    // isolated-actor, async, critical, channel or removed XOR operator syntax.
    for source in [
        include_str!("parity/reject_critical.ores"),
        include_str!("parity/reject_channel_select.ores"),
        include_str!("parity/reject_xor_operator.ores"),
    ] {
        fixture(source, &[], false, "");
    }
    fixture_requires_diagnostic(
        include_str!("parity/checked_overflow.ores"),
        "i64 arithmetic overflow",
    );
    fixture_requires_diagnostic(
        include_str!("parity/reject_actor.ores"),
        "DECL_KIND_UNSUPPORTED: actor",
    );
    fixture_requires_diagnostic(
        include_str!("parity/reject_isoactor.ores"),
        "Cranelift source actor hierarchy, traits and mailbox runtime unsupported",
    );
    fixture_requires_diagnostic(
        include_str!("parity/reject_async.ores"),
        "E_CALLABLE_MODIFIERS",
    );
}

#[test]
fn actor_mutable_state_container_must_not_lower_as_an_unsafe_scalar() {
    // Cranelift stage-0 has no actor runtime, sealed mutable state, protected
    // read/write loans or actor scheduler. This is an intentional negative:
    // never count a scalar JIT result as execution of the shared .ores guest.
    let source = r#"define actor MutableState as
      state { revision: int = 0; }
      receive(ActorMail<int> mail): void {
        self.handle_state(critical[write] |s| -> {
          s.revision = s.revision + 1;
        });
        self.handle_state(critical[read] |s| -> {
          stdio.stdout.write(s.revision);
        });
      }
    end
    pub async routine main(): void {
      val actor = spawn MutableState();
      await actor.get_ready();
    }
    "#;
    fixture_requires_diagnostic(source, "DECL_KIND_UNSUPPORTED: actor");
}

#[test]
fn isolated_actor_heap_admission_fails_closed_in_run_and_ir() {
    // This scalar JIT has no per-actor native heap, ownership lowering,
    // actor scheduler, or compiled .ores state. Refuse canonical forms before
    // emitting IR or running code; a generic parse error is not sufficient.
    let sources = [
        "define isolated actor Worker as state { value: int = 0; } end\n",
        "define pub isolated actor Worker as state { value: int = 0; } end\n",
        "pub define isolated actor Worker as state { value: int = 0; } end\n",
        "define actor Worker as state { value: int = 0; } end\n",
        "pub define actor Worker as state { value: int = 0; } end\n",
        "isoactor Worker { pub fnc get(): int { return 1; } }\n",
        "// a comment preceding an actor is not a scalar escape\ndefine isolated actor Worker as state { x: int = 0; } end\n",
    ];
    for (index, source) in sources.iter().enumerate() {
        let path = std::env::temp_dir().join(format!(
            "orescranelift-heap-admission-{}-{index}.ores",
            std::process::id()
        ));
        std::fs::write(&path, source).unwrap();
        for mode in ["run", "ir"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(1),
                "{mode} accepted actor heap source index {index}"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} emitted guest output or IR for actor source index {index}"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(if source.contains("isolated actor") {
                    "actor runtime is unsupported by the Cranelift scalar profile"
                } else if source.starts_with("isoactor") {
                    "Cranelift source actor hierarchy, traits and mailbox runtime unsupported"
                } else {
                    "DECL_KIND_UNSUPPORTED: actor"
                }),
                "{mode} produced wrong semantic diagnostic for index {index}: {stderr}"
            );
            assert!(!stderr.contains("panic"), "{mode} crashed: {stderr}");
        }
        std::fs::remove_file(path).unwrap();
    }
    fixture(
        "fnc main():int{return 7;} // define isolated actor not live source\n",
        &[],
        true,
        "7\n",
    );
}

fn fixture_requires_diagnostic(source: &str, diagnostic: &str) {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-parity-{}-{sequence}.ores",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(output.status.code(), Some(1), "negative fixture must fail");
    assert!(
        output.stdout.is_empty(),
        "negative fixture cannot emit success stdout"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(diagnostic),
        "wrong negative diagnostic: expected {diagnostic:?}, got {stderr:?}"
    );
    assert!(
        !stderr.contains("panic"),
        "compiler crashed instead of rejecting source"
    );
}
