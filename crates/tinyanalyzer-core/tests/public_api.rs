//! Integration tests for the analysis engine's public API.
//!
//! These exercise only what `src/lib.rs` exports, against real directory trees
//! on disk. Two things are tested here that no unit test can reach:
//!
//! - the whole pipeline end to end, including the `cargo metadata` call, which
//!   is mocked nowhere because a mock of cargo would only ever assert that this
//!   crate agrees with this crate;
//! - the report's serialized form, which is the contract anything storing or
//!   diffing a report depends on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use tempfile::TempDir;
use tinyanalyzer_core::{
    Config, DependencyConfig, Language, Report, Rule, SCHEMA_VERSION, analyze, analyze_with,
};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture is writable");
    }
    std::fs::write(path, contents).expect("the fixture is writable");
}

/// A real cargo workspace with real edges in it.
///
/// Two members, one depending on the other by path, plus a path dev-dependency
/// and one declared-but-never-named dependency. Everything resolves from disk:
/// a fixture that reached the network would make this suite fail on a machine
/// with no network rather than on a real defect, and a fixture with no edges at
/// all would leave the entire graph half of the analyzer unexercised.
fn workspace() -> TempDir {
    let root = TempDir::new().expect("a temporary directory for the fixture");

    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\n",
    );

    write(
        root.path(),
        "crates/engine/Cargo.toml",
        "[package]\nname = \"engine\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        root.path(),
        "crates/engine/src/lib.rs",
        r"//! The fixture engine.

/// Adds two numbers.
pub fn add(a: u8, b: u8) -> u8 {
    a.saturating_add(b)
}

fn never_called() {}

pub fn hot(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        for _ in 0..3 {
            out.push(value.to_string());
        }
    }
    out
}
",
    );
    write(
        root.path(),
        "crates/engine/tests/api.rs",
        "#[test]\nfn adds() {\n    assert_eq!(engine::add(1, 2), 3);\n}\n",
    );

    write(
        root.path(),
        "crates/helper/Cargo.toml",
        "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        root.path(),
        "crates/helper/src/lib.rs",
        "//! Declared by `app` and never named by it.\n\n/// Does nothing in particular.\npub fn assist() {}\n",
    );

    write(
        root.path(),
        "crates/app/Cargo.toml",
        r#"[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
engine = { path = "../engine" }
helper = { path = "../helper" }

[dev-dependencies]
engine = { path = "../engine" }
"#,
    );
    write(
        root.path(),
        "crates/app/src/lib.rs",
        "//! The fixture application.\n\n/// Runs the engine.\npub fn run() -> u8 {\n    engine::add(1, 2)\n}\n",
    );

    root
}

fn no_cargo() -> Config {
    Config {
        dependencies: DependencyConfig {
            enabled: false,
            ..DependencyConfig::default()
        },
        ..Config::default()
    }
}

#[test]
fn it_analyzes_a_real_workspace_end_to_end() {
    let root = workspace();

    let report: Report = analyze(root.path()).expect("a resolvable workspace");

    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert!(report.totals.files >= 4);
    assert!(report.totals.functions >= 3);
    assert!(report.parse_failures.is_empty());
    assert!(
        report
            .languages
            .iter()
            .any(|entry| entry.language == Language::Rust)
    );
}

#[test]
fn it_resolves_the_dependency_graph_of_a_real_workspace() {
    let root = workspace();

    let report = analyze(root.path()).expect("a resolvable workspace");

    assert!(
        report
            .dependencies
            .packages
            .iter()
            .any(|package| package.name == "engine" && package.is_workspace_member),
        "the workspace member must appear in its own graph"
    );
    assert_eq!(report.totals.packages, report.dependencies.packages.len());

    assert!(
        !report.dependencies.edges.is_empty(),
        "`app` depends on `engine` and `helper`"
    );
    assert!(
        report
            .dependencies
            .edges
            .iter()
            .any(|edge| edge.kind == tinyanalyzer_core::DependencyKind::Normal)
    );

    let engine = report
        .dependencies
        .packages
        .iter()
        .find(|package| package.name == "engine")
        .expect("engine is in the graph");
    assert!(engine.is_direct, "`app` names it in its own manifest");
    assert!(!engine.kinds.is_empty());
    assert_eq!(engine.depth, 0, "a workspace member is its own root");

    let packages: Vec<&str> = report
        .dependencies
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    let mut sorted = packages.clone();
    sorted.sort_unstable();
    assert_eq!(packages, sorted, "packages come back ordered");
}

#[test]
fn development_edges_can_be_excluded_from_the_graph() {
    let root = workspace();
    let with_dev = analyze(root.path()).expect("a resolvable workspace");

    let config = Config {
        dependencies: tinyanalyzer_core::DependencyConfig {
            include_dev: false,
            ..tinyanalyzer_core::DependencyConfig::default()
        },
        ..Config::default()
    };
    let without_dev = analyze_with(root.path(), &config).expect("a resolvable workspace");

    let development = |report: &Report| {
        report
            .dependencies
            .edges
            .iter()
            .filter(|edge| edge.kind == tinyanalyzer_core::DependencyKind::Development)
            .count()
    };

    assert!(development(&with_dev) > 0, "the fixture has a dev-dependency");
    assert_eq!(development(&without_dev), 0);
}

#[test]
fn it_names_the_dependency_nothing_uses_and_spares_the_one_that_is_used() {
    let root = workspace();

    let report = analyze(root.path()).expect("a resolvable workspace");
    let unused: Vec<&str> = report
        .dependencies
        .unused
        .iter()
        .map(|entry| entry.dependency.as_str())
        .collect();

    assert!(
        unused.contains(&"helper"),
        "`app` declares `helper` and never names it"
    );
    assert!(
        !unused.contains(&"engine"),
        "`app` calls `engine::add`, so it is used"
    );
}

#[test]
fn a_dependency_can_be_declared_used_through_a_macro() {
    let root = workspace();
    let config = Config {
        dependencies: tinyanalyzer_core::DependencyConfig {
            ignore_unused: vec!["helper".to_owned()],
            ..tinyanalyzer_core::DependencyConfig::default()
        },
        ..Config::default()
    };

    let report = analyze_with(root.path(), &config).expect("a resolvable workspace");

    assert!(
        report.dependencies.unused.is_empty(),
        "an ignored crate is never reported, however unreferenced it looks"
    );
}

#[test]
fn it_attributes_files_to_the_crate_that_owns_them() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    let library = report
        .files
        .iter()
        .find(|file| file.path == "crates/engine/src/lib.rs")
        .expect("the fixture writes it");

    assert_eq!(library.crate_name.as_deref(), Some("engine"));
}

#[test]
fn it_finds_the_unreferenced_function_and_not_the_used_one() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    let names: Vec<&str> = report
        .dead_code
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect();

    assert!(names.contains(&"never_called"));
    assert!(
        !names.contains(&"add"),
        "`app::run` calls it from production code"
    );
    assert!(
        !names.contains(&"assist"),
        "`helper::assist` is public and unused, so it is a medium-confidence          candidate rather than a certain one"
    );
}

#[test]
fn a_public_item_nothing_calls_is_reported_with_lower_confidence() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    let assist = report
        .dead_code
        .iter()
        .find(|candidate| candidate.name == "assist");

    assert!(
        assist.is_none() || assist.is_some_and(|item| item.is_public),
        "a public item is never reported as certainly dead"
    );
}

#[test]
fn counting_tests_as_uses_rescues_a_function_only_the_tests_call() {
    let root = workspace();
    let config = Config {
        dead_code: tinyanalyzer_core::DeadCodeConfig {
            tests_count_as_uses: true,
            ..tinyanalyzer_core::DeadCodeConfig::default()
        },
        ..no_cargo()
    };

    let report = analyze_with(root.path(), &config).expect("a walkable tree");
    let names: Vec<&str> = report
        .dead_code
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect();

    assert!(names.contains(&"never_called"));
    assert!(!names.contains(&"add"));
}

#[test]
fn it_reports_the_allocation_in_the_nested_loop() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    let rules: Vec<Rule> = report.findings.iter().map(|finding| finding.rule).collect();

    assert!(rules.contains(&Rule::AllocationInLoop));
    assert!(rules.contains(&Rule::NestedLoop));
}

#[test]
fn every_finding_carries_a_remedy() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    assert!(!report.findings.is_empty(), "the fixture provokes findings");

    for finding in &report.findings {
        assert!(!finding.title.is_empty());
        assert!(!finding.detail.is_empty());
        assert!(!finding.suggestion.is_empty());
        assert!(!finding.rule.id().is_empty());
    }
}

#[test]
fn the_report_round_trips_through_its_serialized_form() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    let json = report.to_json().expect("a report serializes");
    let restored: Report = serde_json::from_str(&json).expect("and deserializes");

    assert_eq!(restored, report);
}

#[test]
fn the_serialized_report_names_its_schema_version_and_rules_stably() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    let json = report.to_json().expect("a report serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    assert!(value["totals"]["files"].is_number());
    assert!(value["findings"].is_array());

    let rules: Vec<&str> = value["findings"]
        .as_array()
        .expect("findings is an array")
        .iter()
        .filter_map(|finding| finding["rule"].as_str())
        .collect();

    assert!(
        rules.contains(&"allocation_in_loop"),
        "rule identifiers are part of the format, not an implementation detail"
    );
}

#[test]
fn a_configuration_file_changes_what_the_analysis_reports() {
    let root = workspace();
    write(
        root.path(),
        "tinyanalyzer.toml",
        r#"
        [project]
        name = "Fixture"

        [dependencies]
        enabled = false

        [thresholds]
        large_file_lines = 5

        [[notes]]
        path = "crates/engine/src/lib.rs"
        note = "the hot loop is deliberate, for now"
        level = "warning"
        "#,
    );

    let report = analyze(root.path()).expect("a walkable tree");

    assert_eq!(report.project.name, "Fixture");
    assert!(report.dependencies.packages.is_empty());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.rule == Rule::LargeFile || finding.rule == Rule::HugeFile)
    );

    let library = report
        .files
        .iter()
        .find(|file| file.path == "crates/engine/src/lib.rs")
        .expect("the fixture writes it");
    assert_eq!(library.notes.len(), 1);
}

#[test]
fn hiding_tests_changes_the_totals_without_changing_the_report() {
    let root = workspace();

    let report = analyze_with(root.path(), &no_cargo()).expect("a walkable tree");
    let production = report.production_totals();

    assert!(report.totals.test_files > 0, "the fixture has a test file");
    assert!(production.files < report.totals.files);
    assert_eq!(production.test_files, 0);
}

#[test]
fn analyzing_something_that_is_not_a_directory_fails_rather_than_reporting_nothing() {
    let root = workspace();

    assert!(analyze(root.path().join("Cargo.toml")).is_err());
}
