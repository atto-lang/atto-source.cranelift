use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// Integration tests run concurrently. A fixture must never overwrite or
// delete a pre-existing path, including a symlink placed by another process.
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
const NATIVE_DEADLINE: Duration = Duration::from_secs(5);
const MAX_CAPTURE: usize = 65536;

// Drain both compiler output streams while the child is running. Otherwise
// an oversized diagnostic can fill the pipe and prevent process exit.
fn drain_bounded(mut pipe: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut overflow = false;
    let mut buf = [0_u8; 8192];
    loop {
        let len = pipe.read(&mut buf)?;
        if len == 0 {
            return Ok((bytes, overflow));
        }
        let available = MAX_CAPTURE.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buf[..len.min(available)]);
        overflow |= len > available;
    }
}

struct GuestFixture(PathBuf);

impl Drop for GuestFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn execute_native_mode(mode: &str, path: &Path, args: &[&str]) -> std::process::Output {
    assert!(matches!(mode, "run" | "ir"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout_pipe = child.stdout.take().expect("captured child stdout");
    let stderr_pipe = child.stderr.take().expect("captured child stderr");
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout_pipe));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr_pipe));
    let deadline = Instant::now() + NATIVE_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Cranelift guest compiler exceeded five-second deadline");
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Cranelift guest child status failed: {error}");
            }
        }
    }
    let status = child.wait().expect("compiler termination status");
    let (stdout, stdout_overflow) = stdout_reader
        .join().expect("stdout reader panicked").expect("stdout read failed");
    let (stderr, stderr_overflow) = stderr_reader
        .join().expect("stderr reader panicked").expect("stderr read failed");
    assert!(
        !stdout_overflow && !stderr_overflow,
        "Cranelift guest exceeded bounded output capture"
    );
    std::process::Output { status, stdout, stderr }
}

fn fixture(source: &str, args: &[&str], ok: bool, expected: &str) {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-{}-{}-{}.ores",
        std::process::id(),
        sequence,
        source.len()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).unwrap();
    // Create the removal guard only AFTER the exclusive open succeeds.
    let source_file = GuestFixture(path);
    file.write_all(source.as_bytes()).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let output = execute_native_mode("run", &source_file.0, args);
    assert_eq!(
        output.status.code(),
        Some(if ok { 0 } else { 1 }),
        "unexpected compiler status: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if ok {
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        assert!(output.stderr.is_empty(), "successful compiler printed diagnostics");
    } else {
        assert!(output.stdout.is_empty(), "rejected compilation printed stdout");
        assert!(
            output.stderr.starts_with(b"orescranelift: "),
            "rejection requires a compiler-owned diagnostic"
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("panicked at"),
            "compiler panic must not masquerade as source rejection"
        );
    }
}

#[test]
fn bounded_native_capture_detects_oversized_diagnostics_without_blocking() {
    let data = vec![b'X'; MAX_CAPTURE + 2048];
    let (captured, overflow) = drain_bounded(&data[..]).unwrap();
    assert_eq!(captured.len(), MAX_CAPTURE);
    assert!(overflow, "oversized diagnostic cannot pass as truncated success");
    let (at_limit, overflow) = drain_bounded(&data[..MAX_CAPTURE]).unwrap();
    assert_eq!(at_limit.len(), MAX_CAPTURE);
    assert!(!overflow, "exact-size diagnostic remains admissible");
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

#[cfg(unix)]
#[test]
fn symlink_to_regular_atto_source_is_rejected_before_compilation_in_both_modes() {
    use std::os::unix::fs::symlink;
    let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "cranelift-regular-target-{}-{id}.atto",
        std::process::id()
    ));
    let alias = file.with_extension("link");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
    let mut opened = options.open(&file).unwrap();
    let owned = GuestFixture(file);
    opened.write_all(b"fnc main():int{return 42;}").unwrap();
    opened.sync_all().unwrap();
    drop(opened);
    symlink(&owned.0, &alias).unwrap();
    for mode in ["run", "ir"] {
        let output = execute_native_mode(mode, &alias, &[]);
        assert_eq!(output.status.code(), Some(1), "{mode}: symlink admitted");
        assert!(output.stdout.is_empty(), "{mode}: emitted guest output or CLIF");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("source must be a regular file"),
            "{mode}: incorrect source admission diagnostic"
        );
    }
    std::fs::remove_file(alias).unwrap();
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
        "orescranelift-injection-denial-{}-{suffix}.ores", std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut opened = options.open(&file).unwrap();
    let source_file = GuestFixture(file);
    opened.write_all(source.as_bytes()).unwrap();
    opened.sync_all().unwrap();
    drop(opened);
    for mode in ["run", "ir"] {
        let output = execute_native_mode(mode, &source_file.0, &[]);
        assert!(!output.status.success(), "{mode} unexpectedly accepted state injection");
        assert!(output.stdout.is_empty(), "{mode} emitted native output");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("@Inject state-container lowering"), "{diagnostic}");
        assert!(diagnostic.contains("unsupported in Cranelift"), "{diagnostic}");
    }

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
