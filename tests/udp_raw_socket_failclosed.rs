//! Native Cranelift stage-0 must not miscompile an unlowered UDP socket API.
//! Invoke the *real* compiler CLI and keep unsupported != guest PASS.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Input(std::path::PathBuf);
impl Input {
    fn new(source: &str) -> Self {
        for _ in 0..64 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-cranelift-udp-{}-{id}.atto", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(source.as_bytes()).unwrap();
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create fixture: {e}"),
            }
        }
        panic!("no available fixture path");
    }
}
impl Drop for Input { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }

#[test]
fn raw_udp_guest_requires_explicit_compiler_lowering() {
    // A compiler that fails every source cannot pass this negative gate.
    let scalar = Input::new("fnc main():int{return 42;}");
    let accepted = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("check").arg(&scalar.0).output().expect("run valid scalar");
    assert_eq!(accepted.status.code(), Some(0),
        "valid scalar baseline must be admitted");
    assert!(accepted.stderr.is_empty());
    // A compiler whose run mode fails for every source must not earn a
    // green unsupported-UDP result. Execute actual stage-0 native scalar.
    let executed = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run").arg(&scalar.0).output().expect("execute real scalar");
    assert_eq!(executed.status.code(), Some(0), "valid native scalar run failed");
    assert_eq!(executed.stdout, b"42\n", "native guest did not return 42");
    assert!(executed.stderr.is_empty());
    for src in [
        "fnc main():int{return network.bind_udp(\"127.0.0.1\",0);}",
        "fnc main():int{return std.net.udp.bind(0);}",
    ] {
        let source = Input::new(src);
        for mode in ["check", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode).arg(&source.0).output()
                .expect("launch actual Cranelift compiler");
            assert_eq!(output.status.code(), Some(1),
                "UDP must fail in the native compiler, not as CLI/runner failure: {mode}: {src}");
            assert!(output.stdout.is_empty(),
                "no guest success output for unsupported raw UDP");
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(diagnostic.starts_with("orescranelift: "),
                "unsupported UDP needs a compiler-owned diagnostic: {diagnostic}");
            assert!(!diagnostic.contains("panicked"), "compiler crash is not an unsupported capability");
        }
    }
}
