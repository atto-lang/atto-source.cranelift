//! Shared real-compiler harness for AttoLang Cranelift integration tests.
//! A subprocess success proves only the trusted stage-0 scalar subset, not
//! actor isolation, platform parity, or source provenance.
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
const DEADLINE: Duration = Duration::from_secs(5);
const CAPTURE_LIMIT: usize = 65_536;

struct GuestSource(PathBuf);

impl GuestSource {
    fn new(source: &[u8]) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "atto-cranelift-test-{}-{id}.atto",
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).expect("exclusive guest source creation");
        // Only a successfully created file may become owned and unlinked.
        let guard = Self(path);
        file.write_all(source).expect("write exact guest bytes");
        file.sync_all().expect("sync guest bytes");
        guard
    }
}

impl Drop for GuestSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn drain_bounded(mut pipe: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut over_limit = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let len = pipe.read(&mut buffer)?;
        if len == 0 {
            return Ok((output, over_limit));
        }
        let available = CAPTURE_LIMIT.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..len.min(available)]);
        over_limit |= len > available;
    }
}

pub(crate) fn run_source(mode: &str, source: &[u8], args: &[&str]) -> Output {
    assert!(matches!(mode, "run" | "ir"), "only real compiler modes permitted");
    let fixture = GuestSource::new(source);
    let mut child = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&fixture.0)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch actual native compiler");
    // Always drain both streams while the child runs to avoid pipe-capacity
    // deadlocks. Never retain more than 64 KiB of either output stream.
    let stdout_pipe = child.stdout.take().expect("captured compiler stdout");
    let stderr_pipe = child.stderr.take().expect("captured compiler stderr");
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout_pipe));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr_pipe));
    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native compiler exceeded five-second deadline");
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native compiler status unavailable: {error}");
            }
        }
    };
    let (stdout, stdout_overflow) = stdout_reader
        .join()
        .expect("stdout reader panicked")
        .expect("read compiler stdout");
    let (stderr, stderr_overflow) = stderr_reader
        .join()
        .expect("stderr reader panicked")
        .expect("read compiler stderr");
    assert!(
        !stdout_overflow && !stderr_overflow,
        "native compiler exceeded 64 KiB per-stream output budget"
    );
    Output { status, stdout, stderr }
}
