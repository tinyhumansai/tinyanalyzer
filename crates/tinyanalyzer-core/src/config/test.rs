//! Unit tests for configuration loading.
//!
//! The defaults are pinned here because they are the report every unconfigured
//! repository gets, and a silent change to one of them changes every number on
//! every dashboard.

use super::{CONFIG_FILE_NAME, CONFIG_FILE_NAME_ALT, Config, NoteLevel, StartView, compile_glob_set};
use crate::error::Error;
use std::path::Path;
use tempfile::TempDir;

fn temp() -> TempDir {
    TempDir::new().expect("a temporary directory for the fixture")
}

#[test]
fn an_unconfigured_root_loads_the_defaults() {
    let root = temp();

    let config = Config::load(root.path()).expect("no file is not an error");

    assert_eq!(config, Config::default());
    assert!(Config::locate(root.path()).is_none());
}

#[test]
fn the_defaults_are_the_documented_ones() {
    let config = Config::default();

    assert_eq!(config.thresholds.large_file_lines, 400);
    assert_eq!(config.thresholds.huge_file_lines, 800);
    assert_eq!(config.thresholds.long_function_lines, 60);
    assert_eq!(config.thresholds.high_complexity, 15);
    assert_eq!(config.thresholds.heavy_dependency_crates, 20);
    assert!(config.scan.respect_gitignore);
    assert!(!config.scan.include_hidden);
    assert!(!config.scan.follow_symlinks);
    assert!(config.dead_code.enabled);
    assert!(!config.dead_code.tests_count_as_uses);
    assert_eq!(config.ui.start_view, StartView::Overview);
    assert!(!config.ui.hide_tests);
    assert_eq!(config.ui.table_rows, 20);
    assert!(config.ui.unicode);
    assert!(config.scan.exclude.iter().any(|glob| glob == "target/**"));
}

#[test]
fn loads_the_primary_file_name() {
    let root = temp();
    std::fs::write(
        root.path().join(CONFIG_FILE_NAME),
        "[thresholds]\nlarge_file_lines = 120\n",
    )
    .expect("the fixture is writable");

    let config = Config::load(root.path()).expect("valid configuration");

    assert_eq!(config.thresholds.large_file_lines, 120);
    assert_eq!(
        Config::locate(root.path()).expect("the file was just written"),
        root.path().join(CONFIG_FILE_NAME)
    );
}

#[test]
fn loads_the_alternative_file_name() {
    let root = temp();
    std::fs::write(
        root.path().join(CONFIG_FILE_NAME_ALT),
        "[ui]\ntable_rows = 40\n",
    )
    .expect("the fixture is writable");

    let config = Config::load(root.path()).expect("valid configuration");

    assert_eq!(config.ui.table_rows, 40);
}

#[test]
fn the_primary_file_name_wins_over_the_alternative() {
    let root = temp();
    std::fs::write(root.path().join(CONFIG_FILE_NAME), "[ui]\ntable_rows = 1\n")
        .expect("the fixture is writable");
    std::fs::write(root.path().join(CONFIG_FILE_NAME_ALT), "[ui]\ntable_rows = 2\n")
        .expect("the fixture is writable");

    assert_eq!(Config::load(root.path()).expect("valid").ui.table_rows, 1);
}

#[test]
fn rejects_a_file_that_is_not_toml() {
    let root = temp();
    std::fs::write(root.path().join(CONFIG_FILE_NAME), "this is not toml {{")
        .expect("the fixture is writable");

    let error = Config::load(root.path()).expect_err("a parse failure");

    assert!(matches!(error, Error::Config { .. }));
    assert!(error.to_string().contains(CONFIG_FILE_NAME));
}

#[test]
fn rejects_an_unknown_key_rather_than_ignoring_it() {
    let root = temp();
    std::fs::write(
        root.path().join(CONFIG_FILE_NAME),
        "[thresholds]\nlarge_file_linez = 10\n",
    )
    .expect("the fixture is writable");

    let error = Config::load(root.path()).expect_err("a typo is a failure, not a default");

    assert!(matches!(error, Error::Config { .. }));
}

#[test]
fn reports_a_missing_explicit_file_as_io() {
    let root = temp();

    let error = Config::from_file(root.path().join("absent.toml")).expect_err("no such file");

    assert!(matches!(error, Error::Io { .. }));
}

#[test]
fn notes_match_by_glob_and_default_to_info() {
    let config: Config = toml::from_str(
        r#"
        [[notes]]
        path = "src/parser/**"
        note = "hand-written parser"

        [[notes]]
        path = "src/legacy.rs"
        note = "scheduled for deletion"
        level = "critical"
        "#,
    )
    .expect("valid configuration");

    let parser = config.notes_for("src/parser/lexer.rs").expect("valid globs");
    assert_eq!(parser.len(), 1);
    assert_eq!(parser[0].level, NoteLevel::Info);

    let legacy = config.notes_for("src/legacy.rs").expect("valid globs");
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].level, NoteLevel::Critical);

    assert!(config.notes_for("src/main.rs").expect("valid globs").is_empty());
}

#[test]
fn an_invalid_note_glob_is_reported_with_its_pattern() {
    let config: Config = toml::from_str(
        r#"
        [[notes]]
        path = "src/**{"
        note = "broken"
        "#,
    )
    .expect("the glob is only compiled on use");

    let error = config.notes_for("src/lib.rs").expect_err("a glob failure");

    assert!(matches!(error, Error::Glob { .. }));
    assert!(error.to_string().contains("src/**{"));
}

#[test]
fn the_display_name_prefers_the_configured_one() {
    let mut config = Config::default();
    config.project.name = Some("Tiny Analyzer".to_owned());

    assert_eq!(config.display_name(Path::new("/tmp/anything")), "Tiny Analyzer");
}

#[test]
fn the_display_name_falls_back_to_the_directory_name() {
    let config = Config::default();

    assert_eq!(config.display_name(Path::new("/tmp/my-repo")), "my-repo");
    assert_eq!(config.display_name(Path::new("/")), "unnamed project");
}

#[test]
fn an_empty_pattern_list_compiles_to_no_opinion() {
    assert!(compile_glob_set(&[]).expect("an empty list is valid").is_none());
}

#[test]
fn a_pattern_list_compiles_to_a_matcher() {
    let set = compile_glob_set(&["*.rs".to_owned(), "*.toml".to_owned()])
        .expect("valid globs")
        .expect("a non-empty list has an opinion");

    assert!(set.is_match("lib.rs"));
    assert!(set.is_match("Cargo.toml"));
    assert!(!set.is_match("README.md"));
}

#[test]
fn an_invalid_pattern_in_a_list_is_rejected() {
    let error = compile_glob_set(&["src/**{".to_owned()]).expect_err("a glob failure");

    assert!(matches!(error, Error::Glob { .. }));
}
