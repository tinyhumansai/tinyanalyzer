//! Unit tests for the Rust source analyzer.
//!
//! Each test pins one measurement against a source snippet small enough to
//! verify by eye, because every threshold in the report is a comparison against
//! these numbers and a quiet off-by-one in the parser would move every ranking
//! on the dashboard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{DefinitionKind, RustFile, analyze};

fn parsed(source: &str) -> RustFile {
    analyze(source).expect("the fixture is valid Rust")
}

#[test]
fn reports_a_parse_failure_with_its_line() {
    let failure = analyze("fn a() {\nfn b(( {}\n").expect_err("invalid Rust");

    assert!(failure.line >= 1);
    assert!(!failure.message.is_empty());
    assert!(failure.to_string().starts_with("line "));
}

#[test]
fn an_empty_file_measures_as_empty() {
    assert_eq!(parsed(""), RustFile::default());
}

#[test]
fn counts_each_kind_of_item() {
    let file = parsed(
        r"
        use std::fmt;
        pub struct S;
        enum E { A }
        trait T {}
        impl S {}
        mod m {}
        const C: u8 = 1;
        static ST: u8 = 1;
        type Alias = u8;
        macro_rules! mac { () => {} }
        fn f() {}
        ",
    );

    assert_eq!(file.items.structs, 1);
    assert_eq!(file.items.enums, 1);
    assert_eq!(file.items.traits, 1);
    assert_eq!(file.items.impls, 1);
    assert_eq!(file.items.modules, 1);
    assert_eq!(file.items.consts, 1);
    assert_eq!(file.items.statics, 1);
    assert_eq!(file.items.type_aliases, 1);
    assert_eq!(file.items.macros, 1);
    assert_eq!(file.items.functions, 1);
    assert_eq!(file.items.imports, 1);
    assert_eq!(file.items.total(), 10);
}

#[test]
fn counts_public_items_only_when_they_are_public() {
    let file = parsed("pub fn a() {} fn b() {} pub struct S; struct T;");

    assert_eq!(file.public_items, 2);
}

#[test]
fn a_method_is_qualified_by_its_type() {
    let file = parsed("struct Parser; impl Parser { pub fn new() -> Self { Parser } }");

    assert_eq!(file.functions.len(), 1);
    assert_eq!(file.functions[0].name, "new");
    assert_eq!(file.functions[0].qualified_name, "Parser::new");
    assert!(file.functions[0].is_public);
}

#[test]
fn a_free_function_is_not_qualified() {
    let file = parsed("fn run() {}");

    assert_eq!(file.functions[0].qualified_name, "run");
}

#[test]
fn a_function_reports_its_line_span() {
    let file = parsed("\n\nfn run() {\n    let a = 1;\n}\n");

    assert_eq!(file.functions[0].start_line, 3);
    assert_eq!(file.functions[0].end_line, 5);
    assert_eq!(file.functions[0].lines(), 3);
}

#[test]
fn a_straight_line_function_has_complexity_one() {
    let file = parsed("fn run() { let a = 1; let b = a; }");

    assert_eq!(file.functions[0].complexity, 1);
}

#[test]
fn every_branching_construct_adds_complexity() {
    let file = parsed(
        r"
        fn run(a: bool, b: bool) -> Option<u8> {
            if a { }
            while b { }
            for _ in 0..3 { }
            loop { break; }
            match a { true => {}, false => {} }
            let _ = a && b;
            let _ = a || b;
            Some(1u8)?;
            Some(0)
        }
        ",
    );

    // 1 base + if + while + for + loop + 2 match arms + && + || + ?
    assert_eq!(file.functions[0].complexity, 10);
}

#[test]
fn a_nested_function_does_not_inflate_its_parent() {
    let file = parsed("fn outer() { fn inner(a: bool) { if a {} } }");

    let outer = file
        .functions
        .iter()
        .find(|function| function.name == "outer")
        .expect("the outer function is recorded");

    assert_eq!(outer.complexity, 1);
}

#[test]
fn reports_parameter_counts_including_self() {
    let file = parsed("struct S; impl S { fn m(&self, a: u8, b: u8) {} }");

    assert_eq!(file.functions[0].parameters, 3);
}

#[test]
fn recognizes_async_generic_and_unsafe_functions() {
    let file = parsed("pub async fn a<T>(t: T) {} pub unsafe fn b() {}");

    let a = &file.functions[0];
    assert!(a.is_async && a.is_generic && !a.is_unsafe);

    let b = &file.functions[1];
    assert!(b.is_unsafe && !b.is_async && !b.is_generic);

    assert_eq!(file.performance.async_functions, 1);
    assert_eq!(file.performance.generic_functions, 1);
    assert_eq!(file.unsafe_blocks, 1);
}

#[test]
fn counts_unsafe_blocks() {
    let file = parsed("fn a() { unsafe { } unsafe { } }");

    assert_eq!(file.unsafe_blocks, 2);
}

#[test]
fn a_test_function_is_marked_as_one() {
    let file = parsed("#[test] fn t() {} fn plain() {}");

    let test = &file.functions[0];
    let plain = &file.functions[1];

    assert!(test.is_test);
    assert!(!plain.is_test);
}

#[test]
fn everything_inside_a_cfg_test_module_is_test_code() {
    let file = parsed("#[cfg(test)] mod test { fn helper() {} struct Fixture; }");

    assert!(file.definitions.iter().all(|item| item.is_test));
}

#[test]
fn a_file_of_only_test_items_is_a_test_module() {
    let file = parsed("#[cfg(test)] mod test { fn a() {} }");

    assert!(file.is_test_module);
}

#[test]
fn a_file_with_production_items_is_not_a_test_module() {
    let file = parsed("pub fn a() {} #[cfg(test)] mod test { fn b() {} }");

    assert!(!file.is_test_module);
}

#[test]
fn records_definitions_with_their_kind_and_line() {
    let file = parsed("pub struct S;\nfn f() {}\nconst C: u8 = 1;\n");

    let kinds: Vec<DefinitionKind> = file.definitions.iter().map(|item| item.kind).collect();
    assert!(kinds.contains(&DefinitionKind::Struct));
    assert!(kinds.contains(&DefinitionKind::Function));
    assert!(kinds.contains(&DefinitionKind::Const));

    let structure = file
        .definitions
        .iter()
        .find(|item| item.name == "S")
        .expect("the struct is recorded");
    assert_eq!(structure.line, 1);
    assert!(structure.is_public);
}

#[test]
fn a_method_is_not_a_dead_code_candidate() {
    let file = parsed("struct S; impl S { fn method(&self) {} }");

    assert!(
        !file.definitions.iter().any(|item| item.name == "method"),
        "methods are reached through their type, not by name"
    );
}

#[test]
fn an_abi_export_is_marked_externally_reachable() {
    let file = parsed("#[no_mangle] pub fn entry() {}");

    let entry = file
        .definitions
        .iter()
        .find(|item| item.name == "entry")
        .expect("the export is recorded");

    assert!(entry.is_externally_reachable);
}

#[test]
fn every_definition_kind_has_a_label() {
    for kind in [
        DefinitionKind::Function,
        DefinitionKind::Struct,
        DefinitionKind::Enum,
        DefinitionKind::Trait,
        DefinitionKind::Const,
        DefinitionKind::Static,
        DefinitionKind::TypeAlias,
        DefinitionKind::Macro,
        DefinitionKind::Module,
    ] {
        assert!(!kind.label().is_empty());
    }
}

#[test]
fn collects_referenced_crate_roots_from_imports_and_paths() {
    let file = parsed(
        r"
        use serde::Serialize;
        use crate::local::Thing;
        fn a() { let _ = std::mem::size_of::<u8>(); }
        ",
    );

    assert!(file.referenced_crates.contains(&"serde".to_owned()));
    assert!(file.referenced_crates.contains(&"std".to_owned()));
    assert!(
        !file.referenced_crates.contains(&"crate".to_owned()),
        "`crate` names this crate, not a dependency"
    );
}

#[test]
fn referenced_crates_are_sorted_and_deduplicated() {
    let file = parsed("use serde::Serialize; use serde::Deserialize; use aaa::Thing;");

    assert_eq!(file.referenced_crates, ["aaa", "serde"]);
}

#[test]
fn counts_identifier_occurrences_across_the_whole_token_stream() {
    let file = parsed("fn helper() {} fn caller() { helper(); helper(); }");

    assert_eq!(file.identifier_uses.get("helper"), Some(&3));
    assert_eq!(file.identifier_uses.get("caller"), Some(&1));
}

#[test]
fn identifiers_inside_a_macro_invocation_still_count() {
    let file = parsed("fn helper() {} fn caller() { println!(\"{}\", helper()); }");

    assert_eq!(
        file.identifier_uses.get("helper"),
        Some(&2),
        "the definition plus the use inside the macro"
    );
}

#[test]
fn counts_allocating_and_panicking_calls() {
    let file = parsed(
        r#"
        fn a(s: &str, v: &[u8]) {
            let _ = s.to_string();
            let _ = s.to_owned();
            let _ = v.to_vec();
            let _ = format!("{s}");
            let _ = s.chars().collect::<Vec<_>>();
            let _ = Some(1).unwrap();
            let _ = Some(1).expect("present");
            let _ = s.to_string().clone();
        }
        "#,
    );

    assert_eq!(file.performance.allocating_conversions, 5);
    assert_eq!(file.performance.collects, 1);
    assert_eq!(file.performance.unwraps, 2);
    assert_eq!(file.performance.clones, 1);
}

#[test]
fn counts_dynamic_dispatch_sites() {
    let file = parsed("fn a(x: &dyn std::fmt::Debug, y: Box<dyn Fn()>) {}");

    assert_eq!(file.performance.dyn_dispatch, 2);
}

#[test]
fn a_loop_inside_a_loop_is_reported_as_nested() {
    let file = parsed("fn a() { for _ in 0..3 { for _ in 0..3 { } } }");

    assert_eq!(file.performance.nested_loops, 1);
}

#[test]
fn two_sequential_loops_are_not_nested() {
    let file = parsed("fn a() { for _ in 0..3 {} for _ in 0..3 {} }");

    assert_eq!(file.performance.nested_loops, 0);
}

#[test]
fn an_allocation_inside_a_loop_is_reported_separately() {
    let file = parsed(
        r"
        fn a(s: &str) {
            let _ = s.to_string();
            for _ in 0..3 {
                let _ = s.to_string();
            }
        }
        ",
    );

    assert_eq!(file.performance.allocating_conversions, 2);
    assert_eq!(file.performance.allocations_in_loops, 1);
}

#[test]
fn counts_todo_markers_only_in_comments() {
    let file = parsed(
        r#"
        // TODO: split this
        /// FIXME: rename
        fn a() { let _ = "TODO not a comment"; }
        "#,
    );

    assert_eq!(file.todo_markers, 2);
}

#[test]
fn reports_the_deepest_nesting_in_the_file() {
    let file = parsed("fn a(b: bool) { if b { if b { if b { } } } }");

    // The function body plus three nested `if` blocks.
    assert_eq!(file.max_nesting, 4);
    assert_eq!(file.functions[0].max_nesting, 4);
}

#[test]
fn a_trait_method_with_a_default_body_is_measured() {
    let file = parsed("trait T { fn provided(&self) { let _ = 1; } fn required(&self); }");

    assert_eq!(file.items.functions, 2);
    assert_eq!(file.functions.len(), 1);
    assert_eq!(file.functions[0].name, "provided");
}

#[test]
fn item_counts_accumulate() {
    let mut total = super::ItemCounts::default();
    total.add(parsed("fn a() {}").items);
    total.add(parsed("struct S;").items);

    assert_eq!(total.functions, 1);
    assert_eq!(total.structs, 1);
}

#[test]
fn performance_signals_accumulate() {
    let mut total = super::PerformanceSignals::default();
    total.add(parsed("fn a(s: &str) { let _ = s.to_string(); }").performance);
    total.add(parsed("fn b(s: &str) { let _ = s.to_owned(); }").performance);

    assert_eq!(total.allocating_conversions, 2);
}
