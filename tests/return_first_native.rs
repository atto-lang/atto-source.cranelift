//! Native CLI/JIT admission tests for return-first scalar main declarations.
//! Authored regression tests are not evidence of executed CI.
use std::io::Write;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);

// Private source fixtures must never truncate an existing file or follow a
// preexisting symlink. The unique name also separates parallel test processes.
fn guest(source: &str, extension: &str, args: &[&str]) -> Output {
    let sequence = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-return-first-{}-{sequence}.{extension}",
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).expect("exclusive fixture creation");
    file.write_all(source.as_bytes()).expect("write source fixture");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .args(args)
        .output()
        .expect("execute compiled Cranelift guest");
    std::fs::remove_file(path).expect("remove source fixture");
    output
}

fn accepts(source: &str, args: &[&str], expected: &str) {
    for extension in ["ores", "atto"] {
        let output = guest(source, extension, args);
        assert!(
            output.status.success(),
            "{extension} should execute; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "successful guest emitted diagnostics");
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    }
}

fn refuses(source: &str, args: &[&str], diagnostic: &str) {
    for extension in ["ores", "atto"] {
        let output = guest(source, extension, args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{extension} must reject through the compiler, not crash"
        );
        assert!(output.stdout.is_empty(), "rejected guest emitted stdout");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with("orescranelift: "), "{stderr}");
        assert!(stderr.contains(diagnostic), "expected {diagnostic}: {stderr}");
        assert!(!stderr.contains("panicked at"), "compiler panic is not rejection");
    }
}

#[test]
fn return_first_scalar_spellings_execute_compiled_guest() {
    for header in [
        "int <- main()",
        "int• main()",
        "fnc int <- main()",
        "fnc int• main()",
    ] {
        for ending in ["end", "end.", "end;"] {
            let source = format!("{header}\nbegin\n  return 40+2;\n{ending}\n");
            accepts(&source, &[], "42\n");
        }
    }
}

#[test]
fn return_first_parameters_comments_and_line_endings() {
    accepts(
        "int <- main(value:int)\r\nbegin // opener\r\n  return value+1;\r\nend.\r\n",
        &["41"],
        "42\n",
    );
    accepts(
        "fnc int• main(int value)\rbegin\r  return value*2;\rend;\r",
        &["21"],
        "42\n",
    );
}

#[test]
fn return_first_fail_closed_parser_admission() {
    for source in [
        "int <- main() begin\nreturn 42;\nend",
        "int <- main()\nbegin return 42;\nend",
        "int <- main()\u{000b}begin\nreturn 42;\nend",
        "int <- main()\nbegin\u{000b}return 42;\nend",
        "int < - main()\nbegin\nreturn 42;\nend",
        "int <- main()\nbegin\nreturn 42;\nend..",
        "int <- main()\nbegin\nreturn 42;\nend. garbage",
        "int <- main()\nbegin\nreturn 42;",
        "int <- main()\nbegin\nreturn borrow! 42;\nend",
        "int <- main()\nbegin\nreturn 42 as bool;\nend",
        "int <- main()\nbegin\nreturn 42 % 3;\nend",
    ] {
        refuses(source, &[], "");
    }
}

#[test]
fn return_first_checked_failures_never_publish_guest_stdout() {
    refuses(
        "int <- main()\nbegin\nreturn 9223372036854775807+1;\nend",
        &[],
        "overflow",
    );
    refuses(
        "fnc int• main()\nbegin\nreturn Integer.rem(9,0);\nend.",
        &[],
        "divisor zero",
    );
    refuses(
        "int <- main(value:int)\nbegin\nreturn value;\nend",
        &[],
        "runtime argument count mismatch",
    );
}
