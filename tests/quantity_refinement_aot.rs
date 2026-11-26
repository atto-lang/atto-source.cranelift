use std::{fs, process::Command};
#[cfg(unix)]
#[test]
fn checked_refinement_native_objects_preserve_failure_status_and_output() {
    let dir = std::env::temp_dir().join(format!("atto-refinement-aot-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("guest.atto");
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    let binary = dir.join("guest");
    let pre = "define dimension Time; define dimension Length; define unit Second: Time; define unit Meter: Length; define unit Millis: Time = Second / 1000; define unit Huge: Time = Second * 9223372036854775807; define unit Huge2: Time = Huge * 9223372036854775807; type Positive = int where self > 0 and units in Second; type Bounded = int where self >= 1 and self <= 42; type Signed = int where self >= -9223372036854775808 and self <= 9223372036854775807;";
    let compile = |decl: &str, expr: &str| {
        fs::write(
            &source,
            format!("{decl} pub fnc main() -> int {{return {expr};}}"),
        )
        .unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&source)
            .output()
            .unwrap()
    };
    for (expr, status, value) in [
        ("<Positive>(42<Second>)/1<Second>", 0, 42),
        ("(42<Second>)::<Positive>/1<Second>", 0, 42),
        ("((40<Second>+2<Second>) as Positive)/1<Second>", 0, 42),
        ("<Positive>(0<Second>)/1<Second>", 7, 123),
        ("(-1<Second>)::<Positive>/1<Second>", 7, 123),
        ("(0<Second> as Positive)/1<Second>", 7, 123),
        ("<Bounded>(40+2)", 0, 42),
        ("1::<Bounded>", 0, 1),
        ("43 as Bounded", 7, 123),
        ("0::<Bounded>", 7, 123),
        ("<Signed>(-9223372036854775808)", 0, i64::MIN),
        ("<Bounded>(9223372036854775807+1)", 1, 123),
        ("<Bounded>(1<Second>/0<Second>)", 2, 123),
        ("<Bounded>(<Bounded>43)", 7, 123),
        (
            "(0<Huge2>.to_unit<Second>()/1<Second>+42) as Bounded",
            0,
            42,
        ),
        (
            "42000<Millis>.to_unit<Second>()::<Positive>/1<Second>",
            0,
            42,
        ),
    ] {
        let out = compile(pre, expr);
        assert!(
            out.status.success(),
            "{expr}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::write(&object, out.stdout).unwrap();
        let value_c = if value == i64::MIN {
            "INT64_MIN".to_string()
        } else {
            format!("INT64_C({value})")
        };
        fs::write(&harness,format!("#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){{int64_t out=123;assert(oreslang_scalar_entry(0,0,0,0,&out)=={status});assert(out=={value_c});}}")).unwrap();
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
        assert!(Command::new(&binary).status().unwrap().success(), "{expr}");
        println!("REFINEMENT_NATIVE_PASS {expr}");
    }
    for expr in [
        "<Positive>42",
        "42<Millis>::<Positive>",
        "42<Meter> as Positive",
        "42.<Positive>",
        "42<Positive>",
        "<Bounded>(42<Second>)",
        "<Unknown>42",
        "(42<Second>)::<Positive>",
        "42::<Positive>.to_unit<Second>()",
    ] {
        let out = compile(pre, expr);
        assert!(!out.status.success(), "{expr}");
        assert!(out.stdout.is_empty(), "{expr} emitted object");
    }
    for suffix in [
        "type Bad = int where self > 9223372036854775807;",
        "type Bad = int where self < -9223372036854775808;",
        "type Bad = int where self >= 3 and self <= 2;",
        "type Bad = int where self > 0 and units in Missing;",
        "type Bad = i32 where self > 2147483647;",
        "type Bad = int where self > 0 or self < -1;",
        "type Positive = int where self > 1;",
        "type Bad = int where self > = 0;",
        "type Bad = int where self >/*split*/= 0;",
        "type Bad = int where self > 0 units in Second;",
    ] {
        let out = compile(&format!("{pre} {suffix}"), "42");
        assert!(!out.status.success(), "{suffix}");
        assert!(out.stdout.is_empty());
    }
    // The old unsupported-carrier denial is now a real admitted carrier.
    let out = compile("type Narrow = i32 where self > 0;", "42 as Narrow");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.stdout.is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn contracted_delimiters_native_guests() {
    let dir = std::env::temp_dir().join(format!("atto-contract-native-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let input = dir.join("guest.atto");
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    let binary = dir.join("guest");
    let header = "type R = int where self >= 1 and self <= 42; fnc f(int x) -> int requires x >= 1 requires x <= 42";
    let tail = " pub fnc main() -> int { return f(2); }";
    fs::write(&harness,"#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){int64_t out=0;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==42);}\n").unwrap();
    for body in [
        " { return 84/x; }",
        "\nbegin return 84/x; end",
        "\rbegin return 84/x; end",
        " :: return 84/x; end",
        "\n:: return 84/x; end",
    ] {
        fs::write(&input, format!("{header}{body}{tail}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::write(&object, out.stdout).unwrap();
        assert!(
            Command::new("cc")
                .args(["-std=c11", "-Wall", "-Werror"])
                .arg(&harness)
                .arg(&object)
                .arg("-o")
                .arg(&binary)
                .status()
                .unwrap()
                .success()
        );
        assert!(Command::new(&binary).status().unwrap().success());
    }
    for body in [
        " begin return 84/x; end",
        " :: return 84/x; }",
        " :: return 84/x;",
        " :: begin return 84/x; end",
    ] {
        fs::write(&input, format!("{header}{body}{tail}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&input)
            .output()
            .unwrap();
        assert!(!out.status.success() && out.stdout.is_empty(), "{body}");
    }
    // Physical source policy rejects Unicode line separators before lexing.
    fs::write(&input, format!("{header}\u{2028}begin return 84/x; end{tail}")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("emit-object")
        .arg(&input)
        .output()
        .unwrap();
    assert!(!out.status.success() && out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("source_unicode_control"));
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn annotated_contract_native_guests() {
    let dir = std::env::temp_dir().join(format!("atto-annotation-native-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let input = dir.join("guest.atto");
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    let binary = dir.join("guest");
    let header = "type R = int where self >= 1 and self <= 42; @requires(x >= 1)\n@ensures(result <= 84)\nfnc f(int x) -> int requires x <= 42";
    let tail = " pub fnc main() -> int { return f(2); }";
    fs::write(&harness,"#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){int64_t out=0;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==42);}\n").unwrap();
    for body in [
        " { return 84/x; }",
        "\nbegin return 84/x; end",
        " :: return 84/x; end",
        "\n:: return 84/x; end",
    ] {
        fs::write(&input, format!("{header}{body}{tail}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::write(&object, out.stdout).unwrap();
        assert!(
            Command::new("cc")
                .args(["-std=c11", "-Wall", "-Werror"])
                .arg(&harness)
                .arg(&object)
                .arg("-o")
                .arg(&binary)
                .status()
                .unwrap()
                .success()
        );
        assert!(Command::new(&binary).status().unwrap().success());
    }
    for body in [
        " begin return 84/x; end",
        " :: return 84/x; }",
        " :: return 84/x;",
        " :: begin return 84/x; end",
    ] {
        fs::write(&input, format!("{header}{body}{tail}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&input)
            .output()
            .unwrap();
        assert!(!out.status.success() && out.stdout.is_empty(), "{body}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn exclusion_contract_native_guests() {
    let dir = std::env::temp_dir().join(format!("atto-exclusion-native-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let input = dir.join("guest.atto");
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    let binary = dir.join("guest");
    let header = "type R = int where self >= 1 and self <= 42; @requires(x != 0)\nfnc f(int x) -> int requires x != -1";
    let tail = " pub fnc main() -> int { return f(2); }";
    fs::write(&harness,"#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){int64_t out=0;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==42);}\n").unwrap();
    for body in [
        " { return 84/x; }",
        "\nbegin return 84/x; end",
        " :: return 84/x; end",
        "\n:: return 84/x; end",
    ] {
        fs::write(&input, format!("{header}{body}{tail}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::write(&object, out.stdout).unwrap();
        assert!(
            Command::new("cc")
                .args(["-std=c11", "-Wall", "-Werror"])
                .arg(&harness)
                .arg(&object)
                .arg("-o")
                .arg(&binary)
                .status()
                .unwrap()
                .success()
        );
        assert!(Command::new(&binary).status().unwrap().success());
    }
    for body in [
        " begin return 84/x; end",
        " :: return 84/x; }",
        " :: return 84/x;",
        " :: begin return 84/x; end",
    ] {
        fs::write(&input, format!("{header}{body}{tail}")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&input)
            .output()
            .unwrap();
        assert!(!out.status.success() && out.stdout.is_empty(), "{body}");
    }
    for bad in [
        header.replace("requires x != -1", "requires x eq 0"),
        header.replace("requires x != -1", "requires x eq -1"),
        header.to_string() + " requires x >= -2 requires x <= 2",
    ] {
        fs::write(
            &input,
            format!("{bad} :: return 84/x; end pub fnc main() -> int {{ return f(0); }}"),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&input)
            .output()
            .unwrap();
        assert!(!out.status.success() && out.stdout.is_empty());
    }
    fs::remove_dir_all(dir).unwrap();
}
