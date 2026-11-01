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
fn native_scalar_and_unsupported_actor_guest_parity() {
    // Each fixture is actual native Oreslang source, compiled and executed by
    // the Cranelift JIT through its real CLI. Rust is only the test launcher.
    for (source, expected) in [
        (include_str!("parity/checked_precedence.ores"), "28\n"),
        (include_str!("parity/checked_i64_min.ores"), "-9223372036854775808\n"),
        (include_str!("parity/checked_i64_max_minus_one.ores"), "9223372036854775806\n"),
        (include_str!("parity/bitxor_named_intrinsic.ores"), "-9223372036854775808\n"),
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
        "actor runtime is unsupported by the Cranelift scalar profile",
    );
    fixture_requires_diagnostic(
        include_str!("parity/reject_isoactor.ores"),
        "actor runtime is unsupported by the Cranelift scalar profile",
    );
    fixture_requires_diagnostic(
        include_str!("parity/reject_async.ores"),
        "async runtime is unsupported by the Cranelift scalar profile",
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
    fixture_requires_diagnostic(
        source,
        "actor runtime is unsupported by the Cranelift scalar profile",
    );
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
                stderr.contains("actor runtime is unsupported by the Cranelift scalar profile"),
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
    assert!(output.stdout.is_empty(), "negative fixture cannot emit success stdout");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(diagnostic),
        "wrong negative diagnostic: expected {diagnostic:?}, got {stderr:?}"
    );
    assert!(!stderr.contains("panic"), "compiler crashed instead of rejecting source");
}
