use std::{fs,process::Command};
#[cfg(unix)]
#[test]
fn quantity_source_emits_and_links_native_objects(){
 let dir=std::env::temp_dir().join(format!("atto-quantity-aot-{}",std::process::id()));
 fs::create_dir(&dir).unwrap();
 let source=dir.join("guest.atto");let object=dir.join("guest.o");let harness=dir.join("host.c");let binary=dir.join("guest");
 let pre="define dimension Time; define dimension Length; define unit Second: Time; define unit Meter: Length; define unit Tick: Time;";
 let compile=|decl:&str,expr:&str|{
  fs::write(&source,format!("{decl} pub fnc main() -> int {{return {expr};}}")).unwrap();
  Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("emit-object").arg(&source).output().unwrap()
 };
 for (expr,status,value) in [
 ("84.<Second>/2.<Second>",0,42),("84<Second>/2<Tick>",0,42),
 ("84::<Second>/2::<Second>",0,42),("84 as Second / 2 as Second",0,42),
 ("(40.<Meter>+2.<Meter>)/1.<Meter>",0,42),("(44.<Meter>-2.<Meter>)/1.<Meter>",0,42),
 ("(21.<Meter>*2.<Second>)/(1.<Second>*1.<Meter>)",0,42),
 ("(84.<Meter>/2.<Second>)*1.<Second>/1.<Meter>",0,42),
 ("-(-42.<Meter>)/1.<Meter>",0,42),("(<i64>42).<Second>/1.<Second>",0,42),
 ("1.<Second>/0.<Second>",2,123),("(-9223372036854775808.<Second>)/(-1.<Second>)",1,123),
 ("(9223372036854775807.<Second>+1.<Second>)/1.<Second>",1,123)
 ]{
  let out=compile(pre,expr);assert!(out.status.success(),"{expr}: {}",String::from_utf8_lossy(&out.stderr));
  fs::write(&object,out.stdout).unwrap();
  fs::write(&harness,format!("#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){{int64_t out=123;assert(oreslang_scalar_entry(0,0,0,0,&out)=={status});assert(out=={value});}}")).unwrap();
  let link=Command::new("cc").args(["-std=c11","-Wall","-Werror"]).arg(&harness).arg(&object).arg("-o").arg(&binary).output().unwrap();
  assert!(link.status.success(),"{}",String::from_utf8_lossy(&link.stderr));
  assert!(Command::new(&binary).status().unwrap().success(),"{expr}");
  println!("QUANTITY_NATIVE_PASS {expr}");
 }
 for expr in ["1.<Meter>+1.<Second>","1.<Meter>-1","1.<Meter>","1.<Missing>","1.<Time>","1.<Second>.<Second>","1.<Second>::<i64>","<i64>(1.<Second>)","1.<Second> as int","<Second>1","1.<int>","1.<Second>/1.<Meter>","1.<Second>*1.<Second>","42/1","42 : : <i64>","42:/*split*/:<i64>"]{
  let out=compile(pre,expr);assert!(!out.status.success(),"{expr}");assert!(out.stdout.is_empty(),"{expr} emitted native object");
  println!("QUANTITY_NATIVE_REJECT {expr}");
 }
 for decl in ["define dimension Time; define dimension Time;","define unit Second: Unknown;","define dimension Time; define unit Time: Time;","define dimension Time; define unit Second: Time; define unit Second: Time;","define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000;"] {
  let out=compile(decl,"42");assert!(!out.status.success(),"{decl}");assert!(out.stdout.is_empty());
 }
 fs::remove_dir_all(dir).unwrap();
}
