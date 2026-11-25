//! Actor lifecycle target syntax is not executable Cranelift stage-0 code.
//! Negative guest admission only: get_ready(), get_done(), stop! and
//! readiness-free pipe! may not fall back to an unrelated scalar JIT.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn invoke(mode: &str, source: &str) -> Output {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-actor-ready-{}-{nonce}.atto",
        std::process::id()
    ));
    // Never truncate or follow a pre-existing symlink in a shared temp directory.
    // Exclusive creation also prevents collisions between independently running suites.
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options.open(&path).expect("exclusive source fixture");
    handle
        .write_all(source.as_bytes())
        .expect("write source fixture");
    drop(handle);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("run Cranelift CLI");
    std::fs::remove_file(&path).expect("delete guest fixture");
    output
}

#[test]
fn actor_ready_and_physical_done_guests_are_never_emitted_by_scalar_jit() {
    let cases = [
        (
            "explicit-ready-barrier",
            r#"
define actor Worker receives int as
  susp receive(ActorMail<int> mail): void { }
end
pub susp fnc main(): int {
  val worker = spawn Worker();
  await worker.get_ready();
  stop! worker;
  await worker.get_done();
  return 0;
}"#,
        ),
        (
            "readiness-free-pipe-admission",
            r#"
pub fnc main(): int {
  val producer = spawn Producer();
  val consumer = spawn Consumer();
  producer pipe! consumer;
  stop! producer;
  await producer.get_done();
  return 0;
}"#,
        ),
        (
            "reverse-written-pipe",
            r#"
pub fnc main(): int {
  val producer = spawn Producer();
  val consumer = spawn Consumer();
  consumer rpipe! producer;
  return 0;
}"#,
        ),
        ("ready-method", "fnc main():int{return worker.get_ready();}"),
        ("done-method", "fnc main():int{return worker.get_done();}"),
        ("graceful-stop", "fnc main():int{return stop! worker;}"),
        (
            "invalid-ready-property",
            "fnc main():int{return worker.ready;}",
        ),
        (
            "invalid-done-property",
            "fnc main():int{return worker.done;}",
        ),
    ];
    for (name, source) in cases {
        for mode in ["check", "ir", "run"] {
            let out = invoke(mode, source);
            assert!(
                !out.status.success(),
                "{name}/{mode}: actor guest unexpectedly admitted"
            );
            assert!(out.stdout.is_empty(), "{name}/{mode}: emitted value/CLIF");
            let error = String::from_utf8_lossy(&out.stderr);
            assert!(
                error.starts_with("orescranelift: "),
                "{name}/{mode}: {error}"
            );
            assert!(
                !error.to_lowercase().contains("panicked"),
                "{name}/{mode}: {error}"
            );
        }
    }
}

#[test]
fn inert_actor_lifecycle_words_cannot_disable_a_signed_i64_guest() {
    let source = r#"
// stop! worker; await worker.get_ready(); producer pipe! consumer;
pub fnc main():int{return 42;}
"#;
    let out = invoke("run", source);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"42\n");
}
