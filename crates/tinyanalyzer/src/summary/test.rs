//! Unit tests for the text renderer.
//!
//! Rendering is a pure function of a report, so these assert against real
//! analyses of small fixture trees: the text is what a user sees, and asserting
//! on it directly is the only way to notice that a section quietly stopped
//! being printed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{human_bytes, render, truncate_path};
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
fn a_width_of_one_leaves_the_path_alone_rather_than_erasing_it() {
    assert_eq!(truncate_path("src/lib.rs", 1), "src/lib.rs");
}
