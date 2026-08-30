//! Unit tests for the text renderer.
//!
//! Rendering is a pure function of a report, so these assert against real
//! analyses of small fixture trees: the text is what a user sees, and asserting
//! on it directly is the only way to notice that a section quietly stopped
//! being printed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{human_bytes, render, truncate_label, truncate_path};
use std::path::Path;
use tempfile::TempDir;
use tinyanalyzer_core::{Config, DependencyConfig, Report, analyze_with};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture is writable");
    }
    std::fs::write(path, contents).expect("the fixture is writable");
}

fn config() -> Config {
    Config {
        dependencies: DependencyConfig {
            enabled: false,
            ..DependencyConfig::default()
        },
        ..Config::default()
    }
}

fn fixture() -> (TempDir, Report) {
    let root = TempDir::new().expect("a temporary directory for the fixture");
    write(
        root.path(),
        "src/lib.rs",
        &format!(
            "//! docs\npub fn a() {{\n{}}}\n",
            "    let _ = 1;\n".repeat(80)
        ),
    );
    write(root.path(), "src/small.rs", "pub fn b() {}\n");
    write(root.path(), "tests/api.rs", "#[test]\nfn t() {}\n");
    write(root.path(), "README.md", "# Title\n");

    let report = analyze_with(root.path(), &config()).expect("a walkable tree");

    (root, report)
}

#[test]
fn the_header_names_the_project_and_its_root() {
    let (root, report) = fixture();

    let text = render(&report, false);

    assert!(text.contains(&report.project.name));
    assert!(text.contains(&root.path().display().to_string()));
}

#[test]
fn every_section_is_present() {
    let (_root, report) = fixture();

    let text = render(&report, false);

    assert!(text.contains("Totals"));
    assert!(text.contains("Languages"));
    assert!(text.contains("Heaviest files"));
    assert!(text.contains("Findings"));
}

#[test]
fn the_totals_section_says_when_tests_are_excluded() {
    let (_root, report) = fixture();

    assert!(render(&report, true).contains("Totals (excluding tests)"));
    assert!(!render(&report, false).contains("excluding tests"));
}

#[test]
fn hiding_tests_removes_them_from_the_ranking() {
    let (_root, report) = fixture();

    assert!(render(&report, false).contains("tests/api.rs"));
    assert!(!render(&report, true).contains("tests/api.rs"));
}

#[test]
fn the_heaviest_file_is_listed_first() {
    let (_root, report) = fixture();

    let text = render(&report, true);
    let large = text.find("src/lib.rs").expect("the large file is listed");
    let small = text.find("src/small.rs").expect("the small file is listed");

    assert!(large < small);
}

#[test]
fn every_finding_prints_its_measurement_and_its_remedy() {
    let (_root, report) = fixture();
    assert!(!report.findings.is_empty(), "the fixture provokes findings");

    let text = render(&report, false);

    for finding in report.findings.iter().take(10) {
        assert!(text.contains(&finding.title));
        assert!(text.contains(&finding.suggestion));
    }
}

#[test]
fn an_empty_repository_renders_without_panicking() {
    let root = TempDir::new().expect("a temporary directory");
    let report = analyze_with(root.path(), &config()).expect("an empty tree");

    let text = render(&report, false);

    assert!(text.contains("Nothing to report."));
    assert!(text.contains("(none)"));
}

#[test]
fn a_report_without_a_dependency_graph_omits_the_dependency_sections() {
    let (_root, report) = fixture();

    let text = render(&report, false);

    assert!(!text.contains("Heaviest direct dependencies"));
}

/// A real two-crate workspace, so the dependency sections have something to
/// render. Path dependencies only: a fixture that reached the network would
/// fail on a machine with none.
fn workspace() -> (TempDir, Report) {
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
        "//! Engine.\n\n/// Adds.\npub fn add(a: u8, b: u8) -> u8 { a.saturating_add(b) }\n",
    );
    write(
        root.path(),
        "crates/app/Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nengine = { path = \"../engine\" }\n",
    );
    write(
        root.path(),
        "crates/app/src/lib.rs",
        "//! App.\n\n/// Runs.\npub fn run() -> u8 { engine::add(1, 2) }\n",
    );

    let report = analyze_with(root.path(), &Config::default()).expect("a resolvable workspace");

    (root, report)
}

#[test]
fn the_dependency_sections_are_rendered_when_there_is_a_graph() {
    let (_root, report) = workspace();
    assert!(
        !report.dependencies.packages.is_empty(),
        "the fixture resolves"
    );

    let text = render(&report, false);

    assert!(text.contains("Heaviest direct dependencies"));
    assert!(text.contains("engine"));
    assert!(text.contains("external crates"));
}

#[test]
fn duplicated_crates_are_listed_when_there_are_any() {
    let (_root, mut report) = workspace();
    report.dependencies.duplicates = vec![tinyanalyzer_core::DuplicateVersions {
        name: "winnow".to_owned(),
        versions: vec!["0.7.15".to_owned(), "1.0.4".to_owned()],
    }];

    let text = render(&report, false);

    assert!(text.contains("Duplicated crates"));
    assert!(text.contains("0.7.15, 1.0.4"));
}

#[test]
fn a_graph_with_no_direct_dependencies_says_so_rather_than_printing_a_gap() {
    let (_root, mut report) = workspace();
    for package in &mut report.dependencies.packages {
        package.is_direct = false;
    }

    assert!(render(&report, false).contains("(none)"));
}

#[test]
fn unreferenced_items_are_listed_and_the_overflow_is_counted() {
    let (_root, mut report) = workspace();

    report.dead_code = (0..14)
        .map(|index| tinyanalyzer_core::DeadCodeCandidate {
            name: format!("orphan{index}"),
            kind: tinyanalyzer_core::DefinitionKind::Function,
            file: "crates/engine/src/lib.rs".to_owned(),
            line: index + 1,
            is_public: false,
            is_test: false,
            confidence: tinyanalyzer_core::Confidence::High,
            reason: "nothing names it".to_owned(),
        })
        .collect();

    let text = render(&report, false);

    assert!(text.contains("Unreferenced items"));
    assert!(text.contains("orphan0"));
    assert!(
        text.contains("and 4 more"),
        "fourteen candidates, ten rows shown"
    );
}

#[test]
fn the_findings_section_counts_what_it_did_not_print() {
    let (_root, mut report) = workspace();
    let template = report
        .findings
        .first()
        .cloned()
        .unwrap_or_else(|| tinyanalyzer_core::Finding {
            rule: tinyanalyzer_core::Rule::LargeFile,
            severity: tinyanalyzer_core::Severity::Medium,
            title: "placeholder".to_owned(),
            detail: "detail".to_owned(),
            suggestion: "suggestion".to_owned(),
            location: None,
            metric: 1.0,
        });
    report.findings = std::iter::repeat_n(template, 13).collect();

    let text = render(&report, false);

    assert!(text.contains("3 more findings"));
    assert!(text.contains("open the dashboard"));
}

#[test]
fn a_configured_description_is_printed_under_the_name() {
    let (_root, mut report) = fixture();
    report.project.description = Some("what it says".to_owned());

    assert!(render(&report, false).contains("what it says"));
}

#[test]
fn bytes_are_rendered_at_human_scale() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(1_024), "1.0 KiB");
    assert_eq!(human_bytes(1_536), "1.5 KiB");
    assert_eq!(human_bytes(1_048_576), "1.0 MiB");
    assert_eq!(human_bytes(1_073_741_824), "1.0 GiB");
}

#[test]
fn a_short_path_is_left_alone() {
    assert_eq!(truncate_path("src/lib.rs", 20), "src/lib.rs");
}

#[test]
fn a_long_path_is_truncated_from_the_front() {
    let truncated = truncate_path("crates/tinyanalyzer-core/src/rust_source/mod.rs", 20);

    assert_eq!(truncated.chars().count(), 20);
    assert!(truncated.starts_with('…'));
    assert!(truncated.ends_with("mod.rs"));
}

#[test]
fn truncation_does_not_split_a_multi_byte_character() {
    let truncated = truncate_path("crates/ünïcödé/päth/file.rs", 12);

    assert_eq!(truncated.chars().count(), 12);
}

#[test]
fn a_label_is_truncated_from_the_end_so_its_front_survives() {
    let truncated = truncate_label("0.9.12+spec-1.1.0", 11);

    assert_eq!(truncated.chars().count(), 11);
    assert!(truncated.starts_with("0.9.12"));
    assert!(truncated.ends_with('\u{2026}'));
}

#[test]
fn a_short_label_is_left_alone() {
    assert_eq!(truncate_label("1.0.4", 11), "1.0.4");
    assert_eq!(truncate_label("0.9.12+spec-1.1.0", 1), "0.9.12+spec-1.1.0");
}

#[test]
fn a_width_of_one_leaves_the_path_alone_rather_than_erasing_it() {
    assert_eq!(truncate_path("src/lib.rs", 1), "src/lib.rs");
}
