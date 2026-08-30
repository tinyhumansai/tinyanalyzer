//! Unit tests for dead-code detection.
//!
//! The cases here are the ones that decide whether the list is worth reading:
//! an item used only from a macro, an item used only from a test, a public item
//! with no caller in the workspace, and two unrelated items sharing a name.

use super::{Confidence, DeadCodeInput, analyze};
use crate::config::DeadCodeConfig;
use crate::rust_source::{RustFile, analyze as parse};

fn parsed(source: &str) -> RustFile {
    parse(source).expect("the fixture is valid Rust")
}

fn candidates(sources: &[(&str, &RustFile, bool)], config: &DeadCodeConfig) -> Vec<String> {
    let inputs: Vec<DeadCodeInput<'_>> = sources
        .iter()
        .map(|(path, rust, is_test_file)| DeadCodeInput {
            path,
            rust,
            is_test_file: *is_test_file,
        })
        .collect();

    analyze(&inputs, config)
        .into_iter()
        .map(|candidate| candidate.name)
        .collect()
}

#[test]
fn a_disabled_detector_reports_nothing() {
    let file = parsed("fn orphan() {}");
    let config = DeadCodeConfig {
        enabled: false,
        ..DeadCodeConfig::default()
    };

    assert!(candidates(&[("src/lib.rs", &file, false)], &config).is_empty());
}

#[test]
fn an_unreferenced_private_function_is_reported_with_high_confidence() {
    let file = parsed("fn orphan() {} fn used() {} fn caller() { used(); }");
    let inputs = [DeadCodeInput {
        path: "src/lib.rs",
        rust: &file,
        is_test_file: false,
    }];

    let found = analyze(&inputs, &DeadCodeConfig::default());

    assert_eq!(found.len(), 2, "orphan and caller are both unreferenced");
    let orphan = found
        .iter()
        .find(|candidate| candidate.name == "orphan")
        .expect("orphan is reported");
    assert_eq!(orphan.confidence, Confidence::High);
    assert_eq!(orphan.file, "src/lib.rs");
    assert_eq!(orphan.line, 1);
    assert!(orphan.reason.contains("orphan"));
}

#[test]
fn a_referenced_item_is_not_reported() {
    let file = parsed("fn used() {} fn caller() { used(); } fn main() { caller(); }");

    assert!(candidates(&[("src/lib.rs", &file, false)], &DeadCodeConfig::default()).is_empty());
}

#[test]
fn a_use_in_another_file_counts() {
    let defining = parsed("pub fn helper() {}");
    let using = parsed("fn run() { crate::helper(); }");

    let found = candidates(
        &[
            ("src/lib.rs", &defining, false),
            ("src/run.rs", &using, false),
        ],
        &DeadCodeConfig::default(),
    );

    assert!(!found.contains(&"helper".to_owned()));
}

#[test]
fn an_item_used_only_inside_a_macro_is_not_reported() {
    let file = parsed(r#"fn helper() -> u8 { 1 } fn caller() { println!("{}", helper()); }"#);

    let found = candidates(&[("src/lib.rs", &file, false)], &DeadCodeConfig::default());

    assert!(
        !found.contains(&"helper".to_owned()),
        "a token census sees through macro invocations; that is the point of it"
    );
}

#[test]
fn a_public_item_with_no_caller_is_only_medium_confidence() {
    let file = parsed("pub fn exported() {}");
    let inputs = [DeadCodeInput {
        path: "src/lib.rs",
        rust: &file,
        is_test_file: false,
    }];

    let found = analyze(&inputs, &DeadCodeConfig::default());

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].confidence, Confidence::Medium);
    assert!(found[0].is_public);
    assert!(found[0].reason.contains("outside"));
}

#[test]
fn results_are_ordered_with_the_most_certain_first() {
    let file = parsed("pub fn exported() {} fn private_orphan() {}");
    let inputs = [DeadCodeInput {
        path: "src/lib.rs",
        rust: &file,
        is_test_file: false,
    }];

    let found = analyze(&inputs, &DeadCodeConfig::default());

    assert_eq!(found[0].confidence, Confidence::High);
    assert_eq!(found[0].name, "private_orphan");
}

#[test]
fn a_use_from_a_test_file_does_not_rescue_an_item_by_default() {
    let library = parsed("pub fn only_tested() {}");
    let tests = parsed("fn t() { only_tested(); }");

    let found = candidates(
        &[
            ("src/lib.rs", &library, false),
            ("tests/api.rs", &tests, true),
        ],
        &DeadCodeConfig::default(),
    );

    assert!(
        found.contains(&"only_tested".to_owned()),
        "an item only its own tests use is dead weight in the shipped binary"
    );
}

#[test]
fn tests_can_be_configured_to_count_as_uses() {
    let library = parsed("pub fn only_tested() {}");
    let tests = parsed("fn t() { only_tested(); }");
    let config = DeadCodeConfig {
        tests_count_as_uses: true,
        ..DeadCodeConfig::default()
    };

    let found = candidates(
        &[
            ("src/lib.rs", &library, false),
            ("tests/api.rs", &tests, true),
        ],
        &config,
    );

    assert!(!found.contains(&"only_tested".to_owned()));
}

#[test]
fn ignored_names_are_never_reported() {
    let file = parsed("fn main() {} fn keep_me() {}");
    let config = DeadCodeConfig {
        ignore: vec!["main".to_owned(), "keep_me".to_owned()],
        ..DeadCodeConfig::default()
    };

    assert!(candidates(&[("src/main.rs", &file, false)], &config).is_empty());
}

#[test]
fn an_abi_export_is_never_reported() {
    let file = parsed("#[no_mangle] pub extern \"C\" fn entry() {}");

    assert!(candidates(&[("src/lib.rs", &file, false)], &DeadCodeConfig::default()).is_empty());
}

#[test]
fn a_test_function_is_never_reported() {
    let file = parsed("#[cfg(test)] mod test { #[test] fn checks_something() {} }");

    assert!(candidates(&[("src/lib.rs", &file, false)], &DeadCodeConfig::default()).is_empty());
}

#[test]
fn modules_are_excluded_because_a_namespace_is_not_a_symbol() {
    let file = parsed("mod inner { pub fn used() {} } fn caller() { inner::used(); }");

    let found = candidates(&[("src/lib.rs", &file, false)], &DeadCodeConfig::default());

    assert!(!found.contains(&"inner".to_owned()));
}

#[test]
fn two_items_sharing_a_name_vouch_for_each_other() {
    // `a::shared` is called; `b::shared` is not. The census counts occurrences
    // of the name, not of the item, so the call rescues both.
    let file = parsed(
        "mod a { pub fn shared() {} } mod b { pub fn shared() {} } fn caller() { a::shared(); }",
    );

    let found = candidates(&[("src/lib.rs", &file, false)], &DeadCodeConfig::default());

    assert!(
        !found.contains(&"shared".to_owned()),
        "the census under-reports rather than over-reports on a name collision"
    );
}

#[test]
fn two_unused_items_sharing_a_name_are_both_reported() {
    let file = parsed("mod a { pub fn shared() {} } mod b { pub fn shared() {} }");
    let inputs = [DeadCodeInput {
        path: "src/lib.rs",
        rust: &file,
        is_test_file: false,
    }];

    let found = analyze(&inputs, &DeadCodeConfig::default());

    assert_eq!(
        found.iter().filter(|item| item.name == "shared").count(),
        2,
        "nothing names it, so every definition of it is a candidate"
    );
}

#[test]
fn structs_enums_and_constants_are_all_candidates() {
    let file = parsed("struct S; enum E { A } const C: u8 = 1; type T = u8;");
    let inputs = [DeadCodeInput {
        path: "src/lib.rs",
        rust: &file,
        is_test_file: false,
    }];

    let names: Vec<String> = analyze(&inputs, &DeadCodeConfig::default())
        .into_iter()
        .map(|candidate| candidate.name)
        .collect();

    assert!(names.contains(&"S".to_owned()));
    assert!(names.contains(&"E".to_owned()));
    assert!(names.contains(&"C".to_owned()));
    assert!(names.contains(&"T".to_owned()));
}

#[test]
fn every_confidence_level_has_a_label() {
    assert_eq!(Confidence::High.label(), "high");
    assert_eq!(Confidence::Medium.label(), "medium");
}
