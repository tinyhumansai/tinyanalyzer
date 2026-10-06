//! Unit tests for the clone pipeline: filtering, ranking, and serialization.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{CloneInput, analyze, symbols, symbols_to_json_lines};
use crate::config::{CloneConfig, Thresholds};

/// A function long enough to report, its statements distinct from each other.
fn function(name: &str) -> String {
    format!(
        "fn {name}(source: &Reader, offset: u64) -> Result<u64> {{\n    let header = source.read(offset)?;\n    if header.is_empty() {{ return Err(Error::Empty); }}\n    let mut total = 0;\n    for item in header.iter() {{ total += item.len(); }}\n    let kind = match header[0] {{ 1 => Kind::A, 2 => Kind::B, _ => Kind::C }};\n    log(kind, total);\n    Ok(total as u64)\n}}\n"
    )
}

fn input<'a>(path: &'a str, text: &'a str, is_test_path: bool, editable: bool) -> CloneInput<'a> {
    CloneInput {
        path,
        text,
        is_test_path,
        editable,
    }
}

#[test]
fn copies_across_files_are_reported_with_their_locations() {
    let (a, b) = (function("load"), function("fetch"));
    let groups = analyze(
        &[input("src/a.rs", &a, false, true), input("src/b.rs", &b, false, true)],
        &CloneConfig::default(),
        &Thresholds::default(),
    );

    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.instances.len(), 2);
    assert_eq!(group.instances[0].file, "src/a.rs");
    assert_eq!(group.instances[0].item.as_deref(), Some("load"));
    assert_eq!((group.instances[0].start_line, group.instances[0].end_line), (1, 9));
    assert_eq!(group.lines, 9);
    assert_eq!(group.lines_saved, 7);
    assert!(group.editable && !group.in_tests && !group.recursive);
    assert_eq!(group.id.len(), 16);
    assert!(group.score > 0.0);
}

#[test]
fn groups_with_no_editable_copy_are_dropped() {
    let (a, b) = (function("load"), function("fetch"));
    let read_only = analyze(
        &[input("v/a.rs", &a, false, false), input("v/b.rs", &b, false, false)],
        &CloneConfig::default(),
        &Thresholds::default(),
    );
    let mixed = analyze(
        &[input("src/a.rs", &a, false, true), input("v/b.rs", &b, false, false)],
        &CloneConfig::default(),
        &Thresholds::default(),
    );

    assert_eq!(read_only.len(), 0);
    assert_eq!(mixed.len(), 1);
    assert!(!mixed[0].editable);
}

#[test]
fn test_code_ranks_lower_and_can_be_excluded() {
    let (a, b) = (function("load"), function("fetch"));
    let production = analyze(
        &[input("src/a.rs", &a, false, true), input("src/b.rs", &b, false, true)],
        &CloneConfig::default(),
        &Thresholds::default(),
    );
    let tests = analyze(
        &[input("tests/a.rs", &a, true, true), input("tests/b.rs", &b, true, true)],
        &CloneConfig::default(),
        &Thresholds::default(),
    );
    let excluded = analyze(
        &[input("tests/a.rs", &a, true, true), input("tests/b.rs", &b, true, true)],
        &CloneConfig {
            include_tests: false,
            ..CloneConfig::default()
        },
        &Thresholds::default(),
    );

    assert!(tests[0].in_tests);
    assert!(tests[0].score < production[0].score);
    assert_eq!(excluded.len(), 0);
}

#[test]
fn groups_are_capped_and_ranked_best_first() {
    let small = "fn s(a: u8) -> u8 {\n    let b = a + 1;\n    let c = b * 2;\n    let d = c - 3;\n    d\n}\n";
    let (a, b) = (function("load"), function("fetch"));
    let thresholds = Thresholds {
        duplicate_min_tokens: 10,
        duplicate_min_lines: 3,
        ..Thresholds::default()
    };
    let inputs = [
        input("src/a.rs", &a, false, true),
        input("src/b.rs", &b, false, true),
        input("src/c.rs", small, false, true),
        input("src/d.rs", small, false, true),
    ];
    let all = analyze(&inputs, &CloneConfig::default(), &thresholds);
    let capped = analyze(
        &inputs,
        &CloneConfig {
            max_groups: 1,
            ..CloneConfig::default()
        },
        &thresholds,
    );

    assert_eq!(all.len(), 2);
    assert!(all[0].score >= all[1].score);
    assert_eq!(all[0].instances[0].file, "src/a.rs");
    assert_eq!(capped.len(), 1);
    assert_eq!(capped[0], all[0]);
}

#[test]
fn a_disabled_fragment_kind_is_not_reported() {
    let (a, b) = (function("load"), function("fetch"));
    let groups = analyze(
        &[input("src/a.rs", &a, false, true), input("src/b.rs", &b, false, true)],
        &CloneConfig {
            fragment_kinds: Vec::new(),
            ..CloneConfig::default()
        },
        &Thresholds::default(),
    );

    assert_eq!(groups.len(), 0);
}

#[test]
fn the_symbol_index_serializes_one_line_per_item() {
    let source = "fn a() {}\nstruct B;\n";
    let records = symbols(&[input("src/lib.rs", source, false, true)]);
    let lines = symbols_to_json_lines(&records).unwrap();

    assert_eq!(records.len(), 2);
    assert_eq!(lines.lines().count(), 2);
    assert!(lines.lines().all(|line| line.starts_with('{') && line.ends_with('}')));
    assert!(lines.contains("\"qualified_name\":\"a\""));
}
