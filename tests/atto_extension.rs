//! AttoLang .atto source admission and legacy .ores compatibility.
//! This validates only the trusted stage-0 Cranelift scalar subset.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
const DEADLINE: Duration = Duration::from_secs(5);
const MAX_CAPTURE: usize = 65536;

struct SourceFixture(PathBuf);

impl Drop for SourceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn create_source(path: &Path, source: &str) -> std::io::Result<SourceFixture> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    // A failed exclusive create must not own or delete an existing path.
    let fixture = SourceFixture(path.to_owned());
    file.write_all(source.as_bytes())?;
    file.sync_all()?;
    Ok(fixture)
}

fn drain_bounded(mut stream: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut result = Vec::new();
    let mut overflow = false;
    let mut buf = [0_u8; 8192];
    loop {
        let read = stream.read(&mut buf)?;
        if read == 0 {
            return Ok((result, overflow));
        }
        let available = MAX_CAPTURE.saturating_sub(result.len());
        result.extend_from_slice(&buf[..read.min(available)]);
        overflow |= read > available;
    }
}

fn native_execution(extension: &str, source: &str) -> Output {
    native_execution_mode(extension, source, "run")
}

fn native_execution_mode(extension: &str, source: &str, mode: &str) -> Output {
    assert!(matches!(extension, "atto" | "ores"));
    assert!(matches!(mode, "run" | "ir"));
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "attolang-cranelift-{}-{id}.{extension}",
        std::process::id()
    ));
    let fixture = create_source(&path, source).expect("exclusive source creation");
    let mut child = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&fixture.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch native Cranelift guest compiler");
    let stdout = child.stdout.take().expect("captured stdout");
    let stderr = child.stderr.take().expect("captured stderr");
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr));
    let deadline = Instant::now() + DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native Cranelift exceeded per-case deadline");
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Cranelift child status unavailable: {error}");
            }
        }
    }
    let status = child.wait().expect("compiler termination status");
    let (stdout, stdout_overflow) = stdout_reader
        .join().expect("stdout reader panicked").expect("stdout read failed");
    let (stderr, stderr_overflow) = stderr_reader
        .join().expect("stderr reader panicked").expect("stderr read failed");
    assert!(!stdout_overflow && !stderr_overflow, "compiler output budget exceeded");
    Output { status, stdout, stderr }
}

#[test]
fn atto_and_legacy_ores_scalar_sources_execute_with_identical_results() {
    for source in [
        "fnc main():int{return (6+1)*6;}",
        "fnc main():int{return Integer.bitXor(40,2);}",
    ] {
        let atto = native_execution("atto", source);
        let ores = native_execution("ores", source);
        for output in [&atto, &ores] {
            assert_eq!(output.status.code(), Some(0));
            assert!(output.stderr.is_empty(), "successful guest had diagnostics");
            assert_eq!(output.stdout, b"42\n");
        }
        assert_eq!(atto.stdout, ores.stdout);
    }
}

#[test]
fn atto_and_ores_both_fail_closed_for_unsupported_guest_syntax() {
    for source in [
        "define actor Unsupported as end",
        "fnc main():int{return 7%3;}",
        "fnc main():int{return 7 ^ 3;}",
        "pub async fnc main():int{return 42;}",
    ] {
        for extension in ["atto", "ores"] {
            let output = native_execution(extension, source);
            assert_eq!(output.status.code(), Some(1), "{extension}: bad rejection");
            assert!(output.stdout.is_empty(), "{extension}: rejected source printed stdout");
            assert!(
                output.stderr.starts_with(b"orescranelift: "),
                "{extension}: rejection lacks compiler-owned diagnostic"
            );
        }
    }
}

#[test]
fn canonical_atto_path_supports_ir_and_native_execution() {
    const SOURCE: &str = "fnc main():int{return (20+1)*2;}";
    let ir = native_execution_mode("atto", SOURCE, "ir");
    assert_eq!(ir.status.code(), Some(0), "AttoLang IR mode must compile");
    assert!(ir.stderr.is_empty(), "successful IR emitted diagnostics");
    assert!(
        String::from_utf8_lossy(&ir.stdout).contains("function"),
        "expected actual Cranelift function IR"
    );
    let guest = native_execution("atto", SOURCE);
    assert_eq!(guest.status.code(), Some(0));
    assert_eq!(guest.stdout, b"42\\n");
    assert!(guest.stderr.is_empty());
}

#[test]
fn source_fixture_refuses_existing_path_without_overwriting_or_deleting_it() {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "attolang-cranelift-collision-{}-{id}.atto",
        std::process::id()
    ));
    let fixture = create_source(&path, "already present").unwrap();
    assert!(create_source(&path, "overwrite attempt").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"already present");
    drop(fixture);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn source_fixture_rejects_preexisting_symlink_without_following_it() {
    use std::os::unix::fs::symlink;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "attolang-cranelift-target-{}-{id}.atto",
        std::process::id()
    ));
    let alias = target.with_extension("link");
    let fixture = create_source(&target, "safe existing target").unwrap();
    symlink(&target, &alias).unwrap();
    assert!(create_source(&alias, "malicious replacement").is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"safe existing target");
    std::fs::remove_file(alias).unwrap();
    drop(fixture);
}
