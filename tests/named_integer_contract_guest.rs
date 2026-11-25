//! A native JIT test of all 26 versioned named-integer reference vectors.
//! A manifest is not execution evidence; this test must actually run on the exact head.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const CONTRACT: &str = include_str!("../contracts/v1/strict-named-integer-arithmetic-v1.json");
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum Expected {
    Value(&'static str),
    Error(&'static str),
}

struct Case {
    id: &'static str,
    operation: &'static str,
    left: &'static str,
    right: &'static str,
    expected: Expected,
}
const CASES: &[Case] = &[
    Case {
        id: "rem_pos_pos",
        operation: "rem",
        left: "9",
        right: "4",
        expected: Expected::Value("1"),
    },
    Case {
        id: "rem_neg_pos",
        operation: "rem",
        left: "-9",
        right: "4",
        expected: Expected::Value("-1"),
    },
    Case {
        id: "rem_pos_neg",
        operation: "rem",
        left: "9",
        right: "-4",
        expected: Expected::Value("1"),
    },
    Case {
        id: "rem_neg_neg",
        operation: "rem",
        left: "-9",
        right: "-4",
        expected: Expected::Value("-1"),
    },
    Case {
        id: "rem_min_minus_one",
        operation: "rem",
        left: "-9223372036854775808",
        right: "-1",
        expected: Expected::Value("0"),
    },
    Case {
        id: "rem_min_max",
        operation: "rem",
        left: "-9223372036854775808",
        right: "9223372036854775807",
        expected: Expected::Value("-1"),
    },
    Case {
        id: "rem_zero",
        operation: "rem",
        left: "9",
        right: "0",
        expected: Expected::Error("DIVISOR_ZERO"),
    },
    Case {
        id: "mod_pos_pos",
        operation: "mod",
        left: "9",
        right: "4",
        expected: Expected::Value("1"),
    },
    Case {
        id: "mod_neg_pos",
        operation: "mod",
        left: "-9",
        right: "4",
        expected: Expected::Value("3"),
    },
    Case {
        id: "mod_pos_neg",
        operation: "mod",
        left: "9",
        right: "-4",
        expected: Expected::Value("1"),
    },
    Case {
        id: "mod_neg_neg",
        operation: "mod",
        left: "-9",
        right: "-4",
        expected: Expected::Value("3"),
    },
    Case {
        id: "mod_min_minus_one",
        operation: "mod",
        left: "-9223372036854775808",
        right: "-1",
        expected: Expected::Value("0"),
    },
    Case {
        id: "mod_neg_one_min",
        operation: "mod",
        left: "-1",
        right: "-9223372036854775808",
        expected: Expected::Value("9223372036854775807"),
    },
    Case {
        id: "mod_min_max",
        operation: "mod",
        left: "-9223372036854775808",
        right: "9223372036854775807",
        expected: Expected::Value("9223372036854775806"),
    },
    Case {
        id: "mod_zero",
        operation: "mod",
        left: "-1",
        right: "0",
        expected: Expected::Error("DIVISOR_ZERO"),
    },
    Case {
        id: "pow_zero_zero",
        operation: "pow",
        left: "0",
        right: "0",
        expected: Expected::Value("1"),
    },
    Case {
        id: "pow_negative_odd",
        operation: "pow",
        left: "-2",
        right: "3",
        expected: Expected::Value("-8"),
    },
    Case {
        id: "pow_min_exact",
        operation: "pow",
        left: "-2",
        right: "63",
        expected: Expected::Value("-9223372036854775808"),
    },
    Case {
        id: "pow_long_max_exponent_neg_one",
        operation: "pow",
        left: "-1",
        right: "9223372036854775807",
        expected: Expected::Value("-1"),
    },
    Case {
        id: "pow_zero_very_large",
        operation: "pow",
        left: "0",
        right: "9223372036854775807",
        expected: Expected::Value("0"),
    },
    Case {
        id: "pow_overflow",
        operation: "pow",
        left: "2",
        right: "63",
        expected: Expected::Error("OVERFLOW"),
    },
    Case {
        id: "pow_negative_exponent",
        operation: "pow",
        left: "2",
        right: "-1",
        expected: Expected::Error("NEGATIVE_EXPONENT"),
    },
    Case {
        id: "xor_zero",
        operation: "bit_xor",
        left: "0",
        right: "0",
        expected: Expected::Value("0"),
    },
    Case {
        id: "xor_small",
        operation: "bit_xor",
        left: "5",
        right: "3",
        expected: Expected::Value("6"),
    },
    Case {
        id: "xor_negative",
        operation: "bit_xor",
        left: "-1",
        right: "0",
        expected: Expected::Value("-1"),
    },
    Case {
        id: "xor_min_max",
        operation: "bit_xor",
        left: "-9223372036854775808",
        right: "9223372036854775807",
        expected: Expected::Value("-1"),
    },
];

struct Fixture(PathBuf);
impl Fixture {
    fn new(contents: &str) -> Self {
        for _ in 0..64 {
            let index = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-cranelift-named-i64-{}-{index}.atto",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(contents.as_bytes())
                        .expect("write exclusive source");
                    file.sync_all().expect("flush exact source");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create source fixture: {e}"),
            }
        }
        panic!("fixture namespace exhausted")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn contract_case(case: &Case) {
    let marker = format!("\"id\": \"{}\"", case.id);
    assert_eq!(
        CONTRACT.matches(marker.as_str()).count(),
        1,
        "missing or duplicate contract ID"
    );
    let start = CONTRACT.find(marker.as_str()).expect("id exists");
    let after = &CONTRACT[start..];
    let end = after.find("\n    }").expect("case boundary not found");
    let record = &after[..end];
    for (name, value) in [
        ("operation", case.operation),
        ("left", case.left),
        ("right", case.right),
    ] {
        assert!(
            record.contains(format!("\"{name}\": \"{value}\"").as_str()),
            "contract drift at {}.{name}",
            case.id
        );
    }
    let expectation = match case.expected {
        Expected::Value(value) => format!("\"value\": \"{value}\""),
        Expected::Error(error) => format!("\"error\": \"{error}\""),
    };
    assert!(
        record.contains(expectation.as_str()),
        "contract result drift at {}",
        case.id
    );
}

#[test]
fn all_strict_named_integer_vectors_run_native_guest_and_ir() {
    assert!(CONTRACT.contains("\"contract\": \"oreslang.strict-named-integer-arithmetic.v1\""));
    assert!(CONTRACT.contains("\"status\": \"reference vectors; backends not yet executed\""));
    assert_eq!(CASES.len(), 26);
    for case in CASES {
        contract_case(case);
        let operation = if case.operation == "bit_xor" {
            "bitXor"
        } else {
            case.operation
        };
        let source = format!(
            "fnc main():int{{return Integer.{operation}({},{});}}\n",
            case.left, case.right
        );
        let fixture = Fixture::new(&source);
        let ir = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("ir")
            .arg(&fixture.0)
            .output()
            .expect("launch actual IR compiler");
        assert_eq!(
            ir.status.code(),
            Some(0),
            "{} IR: {}",
            case.id,
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(
            ir.stderr.is_empty(),
            "unexpected IR diagnostics for {}",
            case.id
        );
        assert!(!ir.stdout.is_empty(), "no CLIF for {}", case.id);

        let run = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&fixture.0)
            .output()
            .expect("launch native Cranelift JIT");
        match case.expected {
            Expected::Value(expected) => {
                assert_eq!(
                    run.status.code(),
                    Some(0),
                    "{}: {}",
                    case.id,
                    String::from_utf8_lossy(&run.stderr)
                );
                assert!(
                    run.stderr.is_empty(),
                    "success emitted diagnostics: {}",
                    case.id
                );
                assert_eq!(
                    run.stdout,
                    format!("{expected}\n").into_bytes(),
                    "incorrect native result: {}",
                    case.id
                );
            }
            Expected::Error(error) => {
                let diagnostic = match error {
                    "DIVISOR_ZERO" => "integer divisor zero",
                    "OVERFLOW" => "i64 arithmetic overflow",
                    "NEGATIVE_EXPONENT" => "negative integer exponent",
                    _ => panic!("unrecognized error category {error}"),
                };
                assert_eq!(
                    run.status.code(),
                    Some(1),
                    "native compiler accepted {}",
                    case.id
                );
                assert!(
                    run.stdout.is_empty(),
                    "rejected guest returned a value: {}",
                    case.id
                );
                let stderr = String::from_utf8_lossy(&run.stderr);
                assert!(
                    stderr.starts_with("orescranelift: ") && stderr.contains(diagnostic),
                    "incorrect error for {}: {stderr}",
                    case.id
                );
                assert!(
                    !stderr.contains("panicked at"),
                    "compiler panic for {}",
                    case.id
                );
            }
        }
    }
}
