use std::fs;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn run(source: &str, mode: &str) -> Output {
    let path=std::env::temp_dir().join(format!("atto-eight-call-{}-{}.atto",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    use std::io::Write;
    file.write_all(source.as_bytes()).unwrap();
    drop(file);
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self){let _=fs::remove_file(&self.0);} }
    let _cleanup=Cleanup(path.clone());
    Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg(mode).arg(path).output().unwrap()
}
#[test]
fn native_internal_seven_and_eight_typed_slots() {
    let seven="pub fnc ready(bool sealed,bool dropped,int a,int b,int c,int d,int e) -> bool { return sealed && dropped && a == 0 && b == 0 && c == 0 && d == 0 && e == 0; } pub fnc main():int { if ready(true,true,0,0,0,0,0) then return 42; fi return 1; }";
    let eight="pub fnc sum(int a,int b,int c,int d,int e,int f,int g,int h):int { return a + b*2 + c*3 + d*4 + e*5 + f*6 + g*7 + h*8; } pub fnc main():int { return sum(1,2,3,4,5,6,7,8); }";
    for (source,expected) in [(seven,"42"),(eight,"204")] {
        let out=run(source,"run");
        assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(),expected);
        let ir=run(source,"ir");
        assert!(ir.status.success(),"{}",String::from_utf8_lossy(&ir.stderr));
        assert!(String::from_utf8_lossy(&ir.stdout).contains("call"));
    }
    let blocked=seven.replace("0,0,0,0,0)","0,0,0,0,1)");
    assert_ne!(blocked,seven);
    let out=run(&blocked,"run");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(),"1");
    for bad in [
        eight.replace("int h)","int h,int i)").replace("7,8)","7,8,9)"),
        eight.replace("7,8)","7)"),
        eight.replace("7,8)","7,true)"),
        seven.replace("true,true,","1,true,"),
    ] {
        for mode in ["check","run","ir"] {
            let out=run(&bad,mode);
            assert!(!out.status.success()&&out.stdout.is_empty(),"{mode}: {bad}");
        }
    }
}
