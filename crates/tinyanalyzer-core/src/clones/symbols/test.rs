//! Unit tests for the symbol index.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::index;
use crate::clones::detect::Parsed;
use crate::clones::syntax::parse;
use crate::clones::types::CloneInput;

fn files(text: &str, editable: bool, is_test_path: bool) -> Vec<Parsed<'_>> {
    vec![Parsed {
        input: CloneInput {
            path: "src/lib.rs",
            text,
            is_test_path,
            editable,
        },
        tree: parse(text).expect("the grammar loads"),
    }]
}

#[test]
fn every_item_gets_a_record_in_source_order() {
    let source = "struct Point { x: f64 }\nimpl Point {\n    fn scale(&self, by: f64, other: &Point) -> Point { todo() }\n}\ntrait Shape { fn area(&self) -> f64; }\nconst LIMIT: u8 = 3;\nmacro_rules! m { () => {} }\n";
    let records = index(&files(source, true, false));
    let names: Vec<(&str, &str)> = records
        .iter()
        .map(|record| (record.kind.as_str(), record.qualified_name.as_str()))
        .collect();

    assert_eq!(
        names,
        [
            ("struct_item", "Point"),
            ("impl_item", "Point"),
            ("function_item", "Point::scale"),
            ("trait_item", "Shape"),
            ("function_signature_item", "Shape::area"),
            ("const_item", "LIMIT"),
            ("macro_definition", "m"),
        ]
    );
}

#[test]
fn a_function_record_carries_its_signature_and_names() {
    let source = "fn scale(&self, by: f64, other: &Point) -> Point {\n    let total = by * other.x;\n    Point::new(total)\n}\n";
    let records = index(&files(source, false, true));
    let record = &records[0];

    assert_eq!(record.name, "scale");
    assert_eq!(record.parameters, ["&self", "f64", "&Point"]);
    assert_eq!(record.return_type.as_deref(), Some("Point"));
    assert_eq!(record.types, ["Point"]);
    // `Point` in `Point::new` is a path segment, which the grammar spells as a value name.
    assert_eq!(
        record.identifiers,
        ["Point", "by", "new", "other", "scale", "total", "x"]
    );
    assert_eq!(record.start_line, 1);
    assert_eq!(record.end_line, 4);
    assert_eq!(record.shape.len(), 16);
    assert!(record.tokens > 10);
    assert!(!record.editable);
    assert!(record.is_test);
}

#[test]
fn same_shapes_share_a_hash() {
    let records = index(&files(
        "fn a(x: u8) -> u8 { x + 1 }\nfn b(y: u8) -> u8 { y + 1 }\nfn c(y: u8) -> u8 { y - 1 }\n",
        true,
        false,
    ));

    assert_eq!(records[0].shape, records[1].shape);
    assert_ne!(records[1].shape, records[2].shape);
    assert_eq!(records[0].parameters, ["u8"]);
}
