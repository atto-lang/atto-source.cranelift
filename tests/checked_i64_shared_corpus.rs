//! Source-to-real-Cranelift-JIT execution of the exact shared 15-case signed-i64
//! corpus. Cranelift's stage-0 compiler accepts one source main() with <=4 i64
//! parameters, not arbitrary named helper definitions: each entry is explicitly
//! adapted to a parameterized source main, never counted as whole-program parity.
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const CASES: &str = include_str!("../contracts/checked-i64-all-backends-v1/cases.tsv");
const EXPECTED_BLOB: &str = "71309fac2ab6b2191a0056c84d78987bd342dce0";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct SourceFile(PathBuf);
impl SourceFile {
    fn new(contents: &str) -> Self {
        for _ in 0..64 {
            let idx = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-cranelift-i64-{}-{idx}.atto", std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut f) => {
                    f.write_all(contents.as_bytes()).expect("write private Atto source");
                    f.sync_all().expect("persist exact source before compiler invocation");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create source fixture: {e}"),
            }
        }
        panic!("cannot reserve unique source fixture");
    }
}
impl Drop for SourceFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// This mapping is intentionally explicit: a single-expression Cranelift JIT
// stage is not a multi-function Java/JS guest compiler. Every original corpus
// expression must match its audited input before its semantics are adapted.
fn adapted(id: &str, expression: &str) -> (&'static str, &'static [&'static str]) {
    const ADD: &str = "pub fnc main(x:int,y:int):int{return x+y;}\n";
    const SUB: &str = "pub fnc main(x:int,y:int):int{return x-y;}\n";
    const MUL: &str = "pub fnc main(x:int,y:int):int{return x*y;}\n";
    const NEG: &str = "pub fnc main(x:int):int{return -x;}\n";
    const REM: &str = "pub fnc main(x:int,y:int):int{return Integer.rem(x,y);}\n";
    match (id,expression) {
        ("safe-zero","sum(0,0)") => (ADD,&["0","0"]),
        ("safe-upper","sum(9223372036854775807,-1)") =>
            (ADD,&["9223372036854775807","-1"]),
        ("safe-lower","subtract(-9223372036854775808,-1)") =>
            (SUB,&["-9223372036854775808","-1"]),
        ("safe-multiply","multiply(-3037000499,3037000499)") =>
            (MUL,&["-3037000499","3037000499"]),
        ("safe-negate","opposite(-42)") => (NEG,&["-42"]),
        ("safe-min-rem","remainder(-9223372036854775808,-1)") =>
            (REM,&["-9223372036854775808","-1"]),
        ("safe-negative-rem","remainder(-7,3)") => (REM,&["-7","3"]),
        ("overflow-add","sum(9223372036854775807,1)") =>
            (ADD,&["9223372036854775807","1"]),
        ("overflow-sub","subtract(-9223372036854775808,1)") =>
            (SUB,&["-9223372036854775808","1"]),
        ("overflow-mul","multiply(3037000500,3037000500)") =>
            (MUL,&["3037000500","3037000500"]),
        ("overflow-negate","opposite(-9223372036854775808)") =>
            (NEG,&["-9223372036854775808"]),
        ("overflow-mul-min","multiply(-9223372036854775808,-1)") =>
            (MUL,&["-9223372036854775808","-1"]),
        ("overflow-cancel","subtract(sum(9223372036854775807,1),1)") =>
            ("pub fnc main(x:int,y:int,z:int):int{return (x+y)-z;}\n",
             &["9223372036854775807","1","1"]),
        ("overflow-argument","sum(opposite(-9223372036854775808),1)") =>
            ("pub fnc main(x:int,y:int):int{return -x+y;}\n",
             &["-9223372036854775808","1"]),
        ("zero-divisor","remainder(1,0)") => (REM,&["1","0"]),
        _ => panic!("unreviewed checked-i64 case/semantic mapping: {id}: {expression}"),
    }
}

#[test]
fn exact_shared_checked_i64_corpus_executes_as_native_jit_guests() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contracts/checked-i64-all-backends-v1/cases.tsv");
    let digest = Command::new("git").arg("hash-object").arg(&path)
        .output().expect("git required for independent source blob verification");
    assert_eq!(digest.status.code(),Some(0),"could not independently hash corpus");
    assert_eq!(String::from_utf8(digest.stdout).unwrap().trim(),EXPECTED_BLOB,
        "source corpus does not match the independently reviewed Git blob");

    let mut seen = HashSet::new();
    let mut good = 0usize;
    let mut overflow = 0usize;
    let mut zero = 0usize;
    assert_eq!(CASES.lines().count(),15,"exact shared corpus size");
    for line in CASES.lines() {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(),4,"unexpected case columns");
        let [id,kind,expression,expected]: [&str;4] =
            fields.try_into().expect("four exact reviewed TSV columns");
        assert!(seen.insert(id),"duplicate corpus ID: {id}");
        let (guest,values) = adapted(id,expression);
        let source = SourceFile::new(guest);
        let compiler = env!("CARGO_BIN_EXE_orescranelift");
        let ir = Command::new(compiler).arg("ir").arg(&source.0)
            .output().expect("run real Cranelift IR compiler");
        assert_eq!(ir.status.code(),Some(0),"{id} IR: {}",
            String::from_utf8_lossy(&ir.stderr));
        assert!(!ir.stdout.is_empty() && ir.stderr.is_empty(),
            "{id}: no valid Cranelift IR produced");

        let run = Command::new(compiler).arg("run").arg(&source.0).args(values)
            .output().expect("execute compiled native Cranelift guest");
        match kind {
            "value" => {
                good += 1;
                let parsed: i64 = expected.parse().expect("signed 64 expected value");
                assert_eq!(run.status.code(),Some(0),"{id}: {}",
                    String::from_utf8_lossy(&run.stderr));
                assert_eq!(run.stdout.as_slice(),format!("{parsed}\n").as_bytes(),"{id} wrong guest value");
                assert!(run.stderr.is_empty(),"{id} value success emitted diagnostics");
            }
            "overflow" | "zero" => {
                let diagnostic = if kind == "overflow" {
                    overflow += 1;
                    assert_eq!(expected,"ORES_I64_OVERFLOW");
                    "i64 arithmetic overflow"
                } else {
                    zero += 1;
                    assert_eq!(expected,"DIVISION_BY_ZERO");
                    "integer divisor zero"
                };
                assert_eq!(run.status.code(),Some(1),"{id}: expected runtime rejection");
                assert!(run.stdout.is_empty(),"{id} may not emit partial success");
                let stderr = String::from_utf8_lossy(&run.stderr);
                assert!(stderr.starts_with("orescranelift: ") &&
                    stderr.contains(diagnostic) &&
                    !stderr.contains("panicked at"),"{id} wrong fault: {stderr}");
            }
            _ => panic!("unsupported corpus outcome: {kind}"),
        }
    }
    assert_eq!((good,overflow,zero),(7,7,1));
    // This is a source-derived single-entry JIT **adapter** test. It is not
    // the unchanged multi-function Java/JS source, compiler-isolate security,
    // full AOT, actor runtime, or exact-head CI certification.
}

#[test]
#[should_panic(expected = "unreviewed checked-i64 case/semantic mapping")]
fn mutated_expression_cannot_reuse_reviewed_case_identity() {
    adapted("safe-zero", "sum(0,1)");
}
