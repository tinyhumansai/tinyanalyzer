//! Unit tests for the command line.
//!
//! The parser is exercised through `clap`'s own `try_parse_from`, so these
//! assert the actual behavior a user gets rather than the shape of the struct.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Cli, Output, View};
use clap::Parser;
use std::path::{Path, PathBuf};
use tinyanalyzer_core::{CONFIG_FILE_NAME, StartView};
use tempfile::TempDir;

fn parse(args: &[&str]) -> Cli {
    let mut argv = vec!["tinyanalyzer"];
    argv.extend_from_slice(args);

    Cli::try_parse_from(argv).expect("the fixture arguments are valid")
}

#[test]
fn the_path_defaults_to_the_current_directory() {
    assert_eq!(parse(&[]).path, PathBuf::from("."));
}

#[test]
fn a_path_can_be_given_positionally() {
    assert_eq!(parse(&["/tmp/repo"]).path, PathBuf::from("/tmp/repo"));
}

#[test]
fn the_default_output_is_the_dashboard() {
    assert_eq!(parse(&[]).output, Output::Dashboard);
}

#[test]
fn the_output_mode_can_be_chosen() {
    assert_eq!(parse(&["--output", "json"]).output, Output::Json);
    assert_eq!(parse(&["-o", "summary"]).output, Output::Summary);
}

#[test]
fn an_unknown_output_mode_is_rejected() {
    assert!(Cli::try_parse_from(["tinyanalyzer", "--output", "html"]).is_err());
}

#[test]
fn the_dead_code_view_is_spelled_in_kebab_case() {
    assert_eq!(parse(&["--view", "dead-code"]).view, Some(View::DeadCode));
}

#[test]
fn every_view_maps_onto_a_start_view() {
    assert_eq!(StartView::from(View::Overview), StartView::Overview);
    assert_eq!(StartView::from(View::Files), StartView::Files);
    assert_eq!(StartView::from(View::Dependencies), StartView::Dependencies);
    assert_eq!(StartView::from(View::DeadCode), StartView::DeadCode);
    assert_eq!(StartView::from(View::Findings), StartView::Findings);
}

#[test]
fn an_unconfigured_run_gets_the_default_configuration() {
    let root = TempDir::new().expect("a temporary directory");
    let cli = parse(&[&root.path().display().to_string()]);

    let config = cli.config().expect("no configuration file is not an error");

    assert_eq!(config, tinyanalyzer_core::Config::default());
}

#[test]
fn a_configuration_file_in_the_root_is_picked_up() {
    let root = TempDir::new().expect("a temporary directory");
    std::fs::write(
        root.path().join(CONFIG_FILE_NAME),
        "[thresholds]\nlarge_file_lines = 42\n",
    )
    .expect("the fixture is writable");

    let cli = parse(&[&root.path().display().to_string()]);

    assert_eq!(cli.config().expect("valid").thresholds.large_file_lines, 42);
}

#[test]
fn an_explicit_configuration_file_wins_over_the_one_in_the_root() {
    let root = TempDir::new().expect("a temporary directory");
    std::fs::write(
        root.path().join(CONFIG_FILE_NAME),
        "[thresholds]\nlarge_file_lines = 42\n",
    )
    .expect("the fixture is writable");

    let elsewhere = root.path().join("other.toml");
    std::fs::write(&elsewhere, "[thresholds]\nlarge_file_lines = 7\n")
        .expect("the fixture is writable");

    let cli = parse(&[
        &root.path().display().to_string(),
        "--config",
        &elsewhere.display().to_string(),
    ]);

    assert_eq!(cli.config().expect("valid").thresholds.large_file_lines, 7);
}

#[test]
fn a_missing_explicit_configuration_file_is_an_error() {
    let cli = parse(&[".", "--config", "/nonexistent/tinyanalyzer.toml"]);

    assert!(cli.config().is_err());
}

#[test]
fn flags_override_the_file_without_replacing_it() {
    let root = TempDir::new().expect("a temporary directory");
    std::fs::write(
        root.path().join(CONFIG_FILE_NAME),
        "[thresholds]\nlarge_file_lines = 42\n",
    )
    .expect("the fixture is writable");

    let cli = parse(&[
        &root.path().display().to_string(),
        "--no-deps",
        "--no-dead-code",
        "--hidden",
        "--no-ignore",
        "--hide-tests",
        "--view",
        "findings",
    ]);
    let config = cli.config().expect("valid");

    assert!(!config.dependencies.enabled);
    assert!(!config.dead_code.enabled);
    assert!(config.scan.include_hidden);
    assert!(!config.scan.respect_gitignore);
    assert!(config.ui.hide_tests);
    assert_eq!(config.ui.start_view, StartView::Findings);
    assert_eq!(
        config.thresholds.large_file_lines, 42,
        "a flag turns off one pass; it does not discard the file"
    );
}

#[test]
fn omitted_flags_leave_the_configuration_alone() {
    let root = TempDir::new().expect("a temporary directory");
    std::fs::write(
        root.path().join(CONFIG_FILE_NAME),
        "[ui]\nhide_tests = true\nstart_view = \"files\"\n",
    )
    .expect("the fixture is writable");

    let cli = parse(&[&root.path().display().to_string()]);
    let config = cli.config().expect("valid");

    assert!(
        config.ui.hide_tests,
        "an absent flag must not overwrite a configured true with false"
    );
    assert_eq!(config.ui.start_view, StartView::Files);
}

#[test]
fn a_write_target_is_optional_and_parsed_as_a_path() {
    assert_eq!(parse(&[]).write, None);
    assert_eq!(
        parse(&["--write", "report.json"]).write.as_deref(),
        Some(Path::new("report.json"))
    );
}

#[test]
fn the_command_line_definition_is_internally_consistent() {
    use clap::CommandFactory;

    Cli::command().debug_assert();
}
