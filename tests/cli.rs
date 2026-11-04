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
