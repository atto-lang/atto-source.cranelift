//! Negative-only field-sensitive borrow admission for the trusted Cranelift scalar JIT.
//! The Rust/Wasm backend's *bounded* disjoint-field support is not a Cranelift
//! feature. Never convert these rejections into a positive parity receipt.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn invoke(mode: &str, source: &str) -> Output {
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "atto-cranelift-field-borrow-{}-{nonce}.atto",
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
    let mut handle = options.open(&file).expect("exclusive source fixture");
    handle
        .write_all(source.as_bytes())
        .expect("write source fixture");
    drop(handle);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&file)
        .output()
        .expect("run native CLI");
    std::fs::remove_file(file).expect("remove source fixture");
    output
}

#[test]
fn disjoint_field_and_nll_guests_do_not_enter_unimplemented_native_lowering() {
    // Positive Rust/Wasm fixtures are deliberately *negative* on Cranelift:
    // https://github.com/atto-lang/atto-source.rs/pull/476
    let cases = [
        (
            "sibling-loan",
            r#"
define class Person as
  pub let string name = "Alice";
  pub let int age = 30;
end
pub fnc main(): int {
  let person = new Person();
  block {
    val name_view = borrow! person.name;
    person.age = 42;
    stdio.println(name_view);
  }
  return person.age;
}"#,
        ),
        (
            "nll-last-use",
            r#"
define class Person as
  pub let string name = "Alice";
  pub let int age = 42;
end
pub fnc main(): int {
  let person = new Person();
  val name_view = borrow! person.name;
  stdio.println(name_view);
  person.name = "Bob";
  return person.age;
}"#,
        ),
        (
            "rhs-moves-owner",
            r#"
define class Person as
  pub let int age = 30;
end
fnc consume_person(Person person): int { return 42; }
pub fnc main(): int {
  let person = new Person();
  person.age = consume_person(take! person);
  return 0;
}"#,
        ),
        (
            "scalar-owner-borrow",
            "fnc main(x:int):int{return borrow! x;}",
        ),
        (
            "unverified-nested-place",
            "fnc main():int{return root.child.age;}",
        ),
    ];
    for (label, source) in cases {
        for mode in ["check", "ir", "run"] {
            let out = invoke(mode, source);
            assert!(
                !out.status.success(),
                "{label}/{mode} was incorrectly admitted"
            );
            assert!(
                out.stdout.is_empty(),
                "{label}/{mode} emitted scalar/IR output"
            );
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.starts_with("orescranelift: "),
                "{label}/{mode}: {stderr}"
            );
            assert!(
                !stderr.to_lowercase().contains("panicked"),
                "{label}/{mode}: {stderr}"
            );
        }
    }
}

#[test]
fn baseline_signed_i64_guest_still_executes() {
    let out = invoke("run", "pub fnc main():int{return 40+2;}");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"42\n");
}
