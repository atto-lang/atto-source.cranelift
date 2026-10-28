use std::process::Command;

fn fixture(source: &str, args: &[&str], ok: bool, expected: &str) {
    let path=std::env::temp_dir().join(format!("orescranelift-{}-{}.ores",
        std::process::id(), source.len()));
    std::fs::write(&path,source).unwrap();
    let output=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run").arg(&path).args(args).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(output.status.success(),ok,"stderr: {}",String::from_utf8_lossy(&output.stderr));
    if ok { assert_eq!(String::from_utf8_lossy(&output.stdout),expected); }
    else {
        assert!(output.stdout.is_empty(),"rejected compilation printed stdout");
        assert!(!output.stderr.is_empty(),"rejection requires a diagnostic");
    }
}
#[test] fn native_execution_and_negative_controls() {
    fixture("fnc main():int{return 2+3*4;}",&[],true,"14\n");
    fixture("fnc main(x:int,y:int):int{return x*3+y;}",&["7","8"],true,"29\n");
    fixture("fnc main(x:int):int{return x+1;}",&["9223372036854775807"],false,"");
    fixture("fnc main(x:int):int{return x-1;}",&["-9223372036854775808"],false,"");
    fixture("fnc main(x:int):int{return x*2;}",&["4611686018427387904"],false,"");
    fixture("fnc main():int{return -9223372036854775808;}",&[],true,"-9223372036854775808\n");
    fixture("fnc main():int{return 1/0;}",&[],false,"");
    fixture("fnc main():int{return 4;} // café",&[],true,"4\n");
    fixture("fnc main():int{return 1;} unsafe",&[],false,"");
}
