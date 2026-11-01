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
    fixture("fnc main():int{return 42;} // tcp pool rt share", &[], true, "42\n");
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
            std::process::id(), sequence, index
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
                "{mode} unexpectedly admitted unsupported source case {index}"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} emitted guest output/IR for unsupported source case {index}"
            );
            assert!(
                !output.stderr.is_empty(),
                "{mode} did not explain unsupported source case {index}"
            );
        }
        std::fs::remove_file(&path).unwrap();
    }
    fixture(
        "fnc main():int{return 17;} // shared readonly tcp slab pin",
        &[], true, "17\n"
    );
}
