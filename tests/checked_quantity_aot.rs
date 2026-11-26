use std::{fs, process::Command};

#[cfg(unix)]
#[test]
fn normalized_quantity_guests_execute_real_linked_objects() {
    let dir = std::env::temp_dir().join(format!("atto-quantity-aot-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("guest.atto");
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    let binary = dir.join("guest");
    fs::write(
        &harness,
        r#"
#include <stdint.h>
#include <assert.h>
#include <stdlib.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(int argc,char**argv) {
  assert(argc==2);
  int64_t out=1234567;
  int expected=atoi(argv[1]);
  int32_t rc=oreslang_scalar_entry(0,0,0,0,&out);
  assert(rc==expected && out==(rc==0?42:1234567));
}
"#,
    )
    .unwrap();
    let pre = "define dimension Time; define dimension Length; define unit Second: Time; define unit Meter: Length; define unit Tick: Time = Second; define unit Millis: Time = Second / 1000; define unit Kilo: Time = 1000 * Second;";
    for (body, status) in [
        ("return 84.<Second> / 2.<Second>;", 0),
        ("return 42000<Millis>.to_unit<Second>()/1<Second>;", 0),
        ("return 42<Second>.to_unit<Millis>()/1000<Millis>;", 0),
        ("return (21<Millis>*2<Kilo>)/(1<Second>*1<Second>);", 0),
        ("return 42+0<Kilo>.to_unit<Millis>()/1<Millis>;", 0),
        ("return -42000<Millis>.to_unit<Second>() / -1<Second>;", 0),
        ("return 84<Second> / 2<Tick>;", 0),
        ("return 84::<Second> / 2::<Second>;", 0),
        ("return (84 as Second) / (2 as Second);", 0),
        ("return (40.<Meter> + 2.<Meter>) / 1.<Meter>;", 0),
        ("return (44.<Meter> - 2.<Meter>) / 1.<Meter>;", 0),
        (
            "return (21.<Meter> * 2.<Second>) / (1.<Second> * 1.<Meter>);",
            0,
        ),
        (
            "return (84.<Meter> / 2.<Second>) * 1.<Second> / 1.<Meter>;",
            0,
        ),
        (
            "return (21.<Meter> * 2.<Meter>) / (1.<Meter> * 1.<Meter>);",
            0,
        ),
        ("return (42::<i64>).<Second> / 1.<Second>;", 0),
        (
            "val distance=84.<Meter>; val time=2.<Second>; val speed=distance/time; return speed*time/2.<Meter>;",
            0,
        ),
        ("val time=1.<Second>/0.<Second>; return 42;", 2),
        ("return 1.<Second>/0.<Second>;", 2),
        ("return -9223372036854775808.<Second> / -1.<Second>;", 1),
        (
            "return (9223372036854775807.<Meter>+1.<Meter>)/1.<Meter>;",
            1,
        ),
    ] {
        fs::write(&source, format!("{pre} pub fnc main() -> int {{ {body} }}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{body}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::write(&object, out.stdout).unwrap();
        let link = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Werror"])
            .arg(&harness)
            .arg(&object)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            link.status.success(),
            "{}",
            String::from_utf8_lossy(&link.stderr)
        );
        assert!(
            Command::new(&binary)
                .arg(status.to_string())
                .status()
                .unwrap()
                .success(),
            "{body}"
        );
        println!("QUANTITY_NATIVE_PASS {body}");
    }
    for (body, category) in [
        ("return 1.<Meter> + 1.<Second>;", "E_UNIT_DIMENSION"),
        ("return 1<Second>+1<Millis>;", "E_UNIT_SCALE"),
        ("return 1<Second>/1<Millis>;", "E_UNIT_ERASURE"),
        (
            "return 1<Millis>.to_unit<Second>()/1<Second>;",
            "E_UNIT_INEXACT",
        ),
        (
            "return 9223372036854775807<Second>.to_unit<Millis>()/1<Millis>;",
            "E_UNIT_RANGE",
        ),
        (
            "return (1<Second>/1<Millis>).<Second>;",
            "E_UNIT_ATTACHMENT",
        ),
        ("return 1<Meter>.to_unit<Second>();", "E_UNIT_DIMENSION"),
        ("return 1.<Meter>/1.<Second>;", "E_UNIT_ERASURE"),
        ("return 1.<Meter>*1.<Meter>;", "E_UNIT_ERASURE"),
        ("return 42.<Time>;", "E_UNIT_NAME"),
        ("return 42.<Missing>;", "E_UNIT_NAME"),
        ("return 42.<Second>.<Second>;", "E_UNIT_ATTACHMENT"),
        ("return <i64>(42.<Second>);", "E_UNIT_ERASURE"),
        ("return 42.<Second>::<i64>;", "E_UNIT_ERASURE"),
        ("return 42.<Second> as i64;", "E_UNIT_ERASURE"),
        (
            "val distance=42.<Meter>; return distance;",
            "E_UNIT_ERASURE",
        ),
        (
            "val distance=42.<Meter>; val distance=42; return 42;",
            "E_UNIT_BINDING",
        ),
        ("return 42 <Second> / 1<Second>;", "unexpected token"),
    ] {
        fs::write(&source, format!("{pre} pub fnc main() -> int {{ {body} }}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&source)
            .output()
            .unwrap();
        assert!(!out.status.success() && out.stdout.is_empty(), "{body}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(category),
            "{body}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        println!("QUANTITY_NATIVE_REJECT {body}");
    }
    fs::remove_dir_all(&dir).unwrap();
}
