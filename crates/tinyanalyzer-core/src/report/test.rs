//! Unit tests for report assembly.
//!
//! These run the whole pipeline against small real directory trees, because
//! assembly is where the modules meet and the interesting failures are the ones
//! that only appear when they do: a file counted twice, a total that disagrees
//! with the rows it sums, a ranking that reorders itself between runs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{SCHEMA_VERSION, analyze, analyze_with, weight};
use crate::config::{Config, DependencyConfig, Note, NoteLevel};
use crate::error::Error;
use crate::loc::{Language, LineCounts};
use crate::rust_source::analyze as parse;
use std::path::Path;
use tempfile::TempDir;

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture is writable");
    }
    std::fs::write(path, contents).expect("the fixture is writable");
}

/// A tree with no `Cargo.toml`, so no cargo invocation is attempted.
fn plain_fixture() -> TempDir {
    let root = TempDir::new().expect("a temporary directory for the fixture");
    write(
        root.path(),
        "src/lib.rs",
        "//! docs\npub fn a() {}\npub fn b() {}\n",
    );
    write(root.path(), "src/deep/inner.rs", "fn hidden() {}\n");
    write(root.path(), "tests/api.rs", "#[test]\nfn t() {}\n");
    write(root.path(), "README.md", "# Title\n\nProse.\n");
    root
}

fn config_without_cargo() -> Config {
    Config {
        dependencies: DependencyConfig {
            enabled: false,
            ..DependencyConfig::default()
        },
        ..Config::default()
    }
}

#[test]
fn rejects_a_root_that_is_not_a_directory() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "a.rs", "fn a() {}\n");

    let error = analyze(root.path().join("a.rs")).expect_err("not a directory");

    assert!(matches!(error, Error::RootNotADirectory { .. }));
}

#[test]
fn an_empty_directory_produces_an_empty_report() {
    let root = TempDir::new().expect("a temporary directory");

    let report = analyze_with(root.path(), &config_without_cargo()).expect("an empty tree");

    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(report.totals.files, 0);
    assert!(report.files.is_empty());
    assert!(report.findings.is_empty());
}

#[test]
fn the_report_names_the_project_and_its_root() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert_eq!(
        report.project.name,
        root.path()
            .file_name()
            .expect("a temporary directory has a name")
            .to_string_lossy()
    );
    assert_eq!(report.project.root, root.path().display().to_string());
    assert!(report.project.config_file.is_none());
}

#[test]
fn a_configured_name_and_description_are_carried_through() {
    let root = plain_fixture();
    let mut config = config_without_cargo();
    config.project.name = Some("Tiny Analyzer".to_owned());
    config.project.description = Some("what it says".to_owned());

    let report = analyze_with(root.path(), &config).expect("a walkable tree");

    assert_eq!(report.project.name, "Tiny Analyzer");
    assert_eq!(report.project.description.as_deref(), Some("what it says"));
}

#[test]
fn totals_agree_with_the_rows_they_sum() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert_eq!(report.totals.files, report.files.len());
    assert_eq!(
        report.totals.lines.total,
        report.files.iter().map(|file| file.lines.total).sum::<usize>()
    );
    assert_eq!(
        report.totals.bytes,
        report.files.iter().map(|file| file.bytes).sum::<u64>()
    );
    assert_eq!(report.totals.directories, report.directories.len());
}

#[test]
fn files_are_ranked_heaviest_first() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "src/small.rs", "fn a() {}\n");
    write(
        root.path(),
        "src/large.rs",
        &format!("fn a() {{\n{}}}\n", "    let _ = 1;\n".repeat(50)),
    );

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert_eq!(report.files[0].path, "src/large.rs");
    assert!(report.files[0].weight > report.files[1].weight);
}

#[test]
fn two_runs_over_an_unchanged_tree_produce_the_same_ranking() {
    let root = plain_fixture();
    let config = config_without_cargo();

    let first = analyze_with(root.path(), &config).expect("a walkable tree");
    let second = analyze_with(root.path(), &config).expect("a walkable tree");

    let paths = |report: &super::Report| -> Vec<String> {
        report.files.iter().map(|file| file.path.clone()).collect()
    };

    assert_eq!(paths(&first), paths(&second));
}

#[test]
fn test_files_are_recognized_by_path() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");
    let tests = report
        .files
        .iter()
        .find(|file| file.path == "tests/api.rs")
        .expect("the fixture writes a test file");

    assert!(tests.is_test);
    assert_eq!(report.totals.test_files, 1);
}

#[test]
fn test_files_are_recognized_by_contents() {
    let root = TempDir::new().expect("a temporary directory");
    write(
        root.path(),
        "src/only_tests.rs",
        "#[cfg(test)]\nmod inner {\n    #[test]\n    fn t() {}\n}\n",
    );

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert!(report.files[0].is_test, "a file of only test items is test code");
}

#[test]
fn production_totals_exclude_test_code() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");
    let production = report.production_totals();

    assert_eq!(production.files, report.totals.files - report.totals.test_files);
    assert_eq!(production.test_files, 0);
    assert!(production.lines.total < report.totals.lines.total);
    assert_eq!(
        production.files,
        report.production_files().count(),
        "the iterator and the totals must agree"
    );
}

#[test]
fn languages_are_aggregated_and_ranked() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert!(
        report
            .languages
            .iter()
            .any(|entry| entry.language == Language::Rust)
    );
    assert!(
        report
            .languages
            .iter()
            .any(|entry| entry.language == Language::Markdown)
    );
    for pair in report.languages.windows(2) {
        assert!(pair[0].lines.code >= pair[1].lines.code);
    }
}

#[test]
fn directories_are_aggregated_and_flagged_when_they_hold_only_tests() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");
    let tests = report
        .directories
        .iter()
        .find(|directory| directory.path == "tests")
        .expect("the fixture writes a tests directory");
    let source = report
        .directories
        .iter()
        .find(|directory| directory.path == "src")
        .expect("the fixture writes a src directory");

    assert!(tests.is_test_only);
    assert!(!source.is_test_only);
    assert_eq!(source.files, 1, "src/deep is its own directory");
}

#[test]
fn a_rust_file_carries_its_parsed_measurements() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "src/lib.rs", "pub fn a() {}\npub fn b() {}\n");

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");
    let rust = report.files[0]
        .rust
        .as_ref()
        .expect("a Rust file that parses");

    assert_eq!(rust.items.functions, 2);
    assert_eq!(report.totals.functions, 2);
}

#[test]
fn a_non_rust_file_carries_no_parsed_measurements() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "README.md", "# Title\n");

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert!(report.files[0].rust.is_none());
}

#[test]
fn an_unparseable_file_is_reported_rather_than_dropped() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "src/broken.rs", "fn a(( {\n");

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert_eq!(report.files.len(), 1, "the file is still counted");
    assert!(report.files[0].rust.is_none());
    assert_eq!(report.parse_failures.len(), 1);
    assert_eq!(report.parse_failures[0].path, "src/broken.rs");
}

#[test]
fn files_are_attributed_to_the_crate_that_owns_them() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    write(
        root.path(),
        "crates/inner/Cargo.toml",
        "[package]\nname = \"inner\"\nversion = \"0.1.0\"\n",
    );
    write(root.path(), "crates/inner/src/lib.rs", "pub fn a() {}\n");
    write(root.path(), "loose.rs", "fn b() {}\n");

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    let owned = report
        .files
        .iter()
        .find(|file| file.path == "crates/inner/src/lib.rs")
        .expect("the fixture writes a crate file");
    let loose = report
        .files
        .iter()
        .find(|file| file.path == "loose.rs")
        .expect("the fixture writes a loose file");

    assert_eq!(owned.crate_name.as_deref(), Some("inner"));
    assert_eq!(
        loose.crate_name, None,
        "a virtual workspace root declares no package"
    );
}

#[test]
fn notes_from_the_configuration_are_attached_to_matching_files() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "src/legacy.rs", "fn a() {}\n");
    write(root.path(), "src/fresh.rs", "fn b() {}\n");

    let mut config = config_without_cargo();
    config.notes = vec![Note {
        path: "src/legacy.rs".to_owned(),
        note: "scheduled for deletion".to_owned(),
        level: NoteLevel::Warning,
    }];

    let report = analyze_with(root.path(), &config).expect("a walkable tree");

    let legacy = report
        .files
        .iter()
        .find(|file| file.path == "src/legacy.rs")
        .expect("the fixture writes it");
    let fresh = report
        .files
        .iter()
        .find(|file| file.path == "src/fresh.rs")
        .expect("the fixture writes it");

    assert_eq!(legacy.notes.len(), 1);
    assert_eq!(legacy.notes[0].level, NoteLevel::Warning);
    assert!(fresh.notes.is_empty());
}

#[test]
fn dead_code_is_found_across_the_whole_tree() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "src/lib.rs", "fn orphan() {}\nfn used() {}\n");
    write(root.path(), "src/other.rs", "fn caller() { used(); }\n");

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");
    let names: Vec<&str> = report
        .dead_code
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect();

    assert!(names.contains(&"orphan"));
    assert!(!names.contains(&"used"));
}

#[test]
fn a_disabled_dependency_pass_leaves_the_graph_empty() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");

    assert!(report.dependencies.packages.is_empty());
    assert_eq!(report.totals.packages, 0);
}

#[test]
fn a_tree_cargo_cannot_resolve_still_produces_a_file_report() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "Cargo.toml", "this is not a manifest {{\n");
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");

    let report = analyze(root.path()).expect("the file half of the analysis still runs");

    assert!(report.dependencies.packages.is_empty());
    assert!(report.files.iter().any(|file| file.path == "src/lib.rs"));
}

#[test]
fn a_configuration_file_in_the_tree_is_found_and_recorded() {
    let root = TempDir::new().expect("a temporary directory");
    write(
        root.path(),
        "tinyanalyzer.toml",
        "[thresholds]\nlarge_file_lines = 1\n\n[dependencies]\nenabled = false\n",
    );
    write(root.path(), "src/lib.rs", "pub fn a() {}\npub fn b() {}\n");

    let report = analyze(root.path()).expect("a walkable tree");

    assert!(
        report
            .project
            .config_file
            .as_deref()
            .is_some_and(|path| path.ends_with("tinyanalyzer.toml"))
    );
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.rule == crate::findings::Rule::LargeFile),
        "the configured threshold of one line must actually take effect"
    );
}

#[test]
fn the_report_serializes_and_round_trips() {
    let root = plain_fixture();

    let report = analyze_with(root.path(), &config_without_cargo()).expect("a walkable tree");
    let json = report.to_json().expect("a report serializes");
    let restored: super::Report = serde_json::from_str(&json).expect("and deserializes");

    assert_eq!(restored, report);
}

#[test]
fn weight_is_lines_alone_for_a_file_with_no_parsed_source() {
    let counts = LineCounts {
        total: 10,
        code: 8,
        comment: 1,
        blank: 1,
    };

    assert!((weight(counts, None) - 8.0).abs() < f64::EPSILON);
}

#[test]
fn weight_rises_with_branching_beyond_the_line_count() {
    let counts = LineCounts {
        total: 1,
        code: 1,
        comment: 0,
        blank: 0,
    };
    let straight = parse("fn a() { let _ = 1; }").expect("valid Rust");
    let branching = parse("fn a(b: bool) { if b { } if b { } }").expect("valid Rust");

    assert!(weight(counts, Some(&branching)) > weight(counts, Some(&straight)));
}

#[test]
fn weight_penalizes_allocation_inside_a_loop() {
    let counts = LineCounts {
        total: 1,
        code: 1,
        comment: 0,
        blank: 0,
    };
    let hoisted = parse("fn a(s: &str) { let _ = s.to_string(); for _ in 0..3 { } }")
        .expect("valid Rust");
    let inside =
        parse("fn a(s: &str) { for _ in 0..3 { let _ = s.to_string(); } }").expect("valid Rust");

    assert!(weight(counts, Some(&inside)) > weight(counts, Some(&hoisted)));
}
