use std::fs;
use std::process::Command;

#[test]
fn actual_named_aggregate_guests_and_rejections() {
    let dir=std::env::temp_dir().join(format!("atto-named-aggregate-{}",std::process::id()));
    fs::create_dir(&dir).expect("isolated guest directory");
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
    let _cleanup=Cleanup(dir.clone());
    let path=dir.join("case.atto");
    let run=|mode:&str,body:&str| {
        fs::write(&path,format!("pub fnc main():int {{ {body} }}")).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg(mode).arg(&path).output().unwrap()
    };
    for body in [
        "const Record<infer R> r=(a=1,b=true);const Tuple<infer T> t=(true,2);const struct<infer S> s={a:3};const Record<R> r2=(b=false,a=10);const Tuple<T> t2=(false,12);const struct<S> s2={a:20};return r2.a+t2[1]+s2.a;",
        "const Record r=(a=10);const Tuple t=(true,12);const struct s={a:20};return r.a+t[1]+s.a;",
        "const Record<infer R> r=(a=-1);const Record<R> s=(a=43);return r.a+s.a;",
    ] {
        let guest=run("run",body);
        assert!(guest.status.success(),"{body}: {:?}",guest);
        assert_eq!(guest.stdout,b"42\n");
        let ir=run("ir",body);
        assert!(ir.status.success(),"{body}: {:?}",ir);
        assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));
    }
    for (body,diagnostic) in [
        ("const Record<infer> r=(a=1);","identifier"),
        ("const Record<R> r=(a=1);","E_UNKNOWN_TYPE"),
        ("const Record<infer R> r=(a=1);const Tuple<infer R> t=(1,2);","E_DUPLICATE_TYPE"),
        ("const Record<infer R> r=(a=1);const Record<R> s=(a=true);","E_TYPE_MISMATCH"),
        ("const Record<infer R> r=(a=1);const Record<R> s=(a=1,b=2);","E_TYPE_MISMATCH"),
        ("const Record<infer R> r=(a=1,b=2);const Record<R> s=(a=1);","E_TYPE_MISMATCH"),
        ("const Record<infer R> r=(a=1,a=2);","E_DUPLICATE_FIELD"),
        ("const Tuple<infer T> t=(1,true);const Tuple<T> s=(true,1);","E_TYPE_MISMATCH"),
        ("const Tuple<infer T> t=(1,true);const Tuple<T> s=(1,);","E_TYPE_MISMATCH"),
        ("const struct<infer S> s={a:1};const Record<S> r=(a=1);","E_AGGREGATE_KIND"),
        ("const Record<infer R> r=(a=unknown);","E_TYPE_INFER_AMBIGUOUS"),
    ] {
        let bad=run("ir",&format!("{body} return 0;"));
        assert!(!bad.status.success(),"{body}");
        assert!(bad.stdout.is_empty(),"{body}");
        assert!(String::from_utf8_lossy(&bad.stderr).contains(diagnostic),"{body}: {:?}",bad);
    }
    // These names must not disappear into scalar constants before admission.
    for alias in ["Pointer", "ActorMail", "ReadLoan", "AtomicInt", "RwLock",
                  "boolean", "u64", "unsafe", "using", "persistent"] {
        let body=format!("const Record<infer {alias}> r=(a=1);return r.a;");
        let bad=run("ir",&body);
        assert!(!bad.status.success(),"{body}: {:?}",bad);
        assert!(bad.stdout.is_empty(),"{body}");
    }
    for body in [
        "const Record<infer R> while=(a=1);return 0;",
        "const struct<infer S> s={await:1};return 0;",
    ] {
        let bad=run("ir",body);
        assert!(!bad.status.success(),"{body}");
        assert!(bad.stdout.is_empty(),"{body}");
    }
    for (body,expected) in [
        ("const Record<infer If> r=(a=1);const Record<If> s=(a=42);return s.a;", b"42\n"),
        ("const Record<infer R> r=(a=1,b=true);const Record<R> s=(b=false,a=11);const Tuple<infer T> t=(true,2);const Tuple<T> t2=(false,14);const struct<infer S> x={a:3};const struct<S> x2={a:23};return s.a+t2[1]+x2.a;", b"48\n"),
    ] {
        let guest=run("run",body);
        assert!(guest.status.success(),"{body}: {:?}",guest);
        assert_eq!(guest.stdout.as_slice(),expected.as_slice());
    }
    for body in [
        "const Record r={a:=40,b:2};return r.a+r.b;",
        "const Record r=(a:=40,b=2,);return r.a+r.b;",
        "const Record r=();const Record s={};const Tuple t=();return 42;",
        "const Tuple t=(42,);return t[0];",
        "const Record<infer R> r={a:1};const Record<R> s=(a:=42);return s.a;",
    ] {
        let guest=run("run",body);
        assert!(guest.status.success(),"{body}: {:?}",guest);
        assert_eq!(guest.stdout,b"42\n");
    }
    for (body,diagnostic) in [
        ("const Tuple t=(1);","E_TUPLE_ARITY"),
        ("const Record r={a:=1,a:2};","E_DUPLICATE_FIELD"),
        ("const Record r={a : = 1};","bool/int literal required"),
        ("const Record<infer R> R=(a=1);","duplicate aggregate local"),
        ("const Record<infer R> r=(a=1);const Record R=(a=2);","duplicate aggregate local"),
        ("const Record<infer R> r=(a=1);const int R=2;","invalid, reserved or duplicate local identifier"),
    ] {
        let bad=run("ir",&format!("{body} return 0;"));
        assert!(!bad.status.success(),"{body}");
        assert!(bad.stdout.is_empty(),"{body}");
        assert!(String::from_utf8_lossy(&bad.stderr).contains(diagnostic),"{body}: {:?}",bad);
    }
    let unsupported=run("ir","return Tuple.of(1,2)[0];");
    assert!(!unsupported.status.success());
    assert!(unsupported.stdout.is_empty());
}
