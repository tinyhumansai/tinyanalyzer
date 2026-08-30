//! Integration tests for the shipped binary.
//!
//! These run the real executable against a real directory tree and read its
//! standard output and exit code. That is the only layer at which the promises
//! in `README.md` are actually testable: that `--output json` emits parseable
//! JSON, that `--output summary` emits readable text, that `--write` puts it in
//! a file, and that a bad path fails loudly instead of reporting an empty
//! repository.
//!
//! The dashboard is deliberately not exercised here. It needs a terminal, and
//! everything it does that is not drawing is unit-tested against its state
//! machine and its renderer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// The binary cargo just built for this test.
const BINARY: &str = env!("CARGO_BIN_EXE_tinyanalyzer");

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture is writable");
    }
    std::fs::write(path, contents).expect("the fixture is writable");
}

fn fixture() -> TempDir {
    let root = TempDir::new().expect("a temporary directory for the fixture");

    write(
        root.path(),
        "tinyanalyzer.toml",
        "[project]\nname = \"Fixture\"\n\n[dependencies]\nenabled = false\n",
    );
    write(
        root.path(),
        "src/lib.rs",
        "//! docs\npub fn add(a: u8, b: u8) -> u8 { a + b }\nfn never_called() {}\n",
    );
    write(root.path(), "tests/api.rs", "#[test]\nfn t() {}\n");

    root
}

fn run(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(BINARY);
    command.arg(root);
    command.args(args);

    command.output().expect("the binary was built for this test")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn it_prints_a_summary_and_exits_successfully() {
    let root = fixture();

    let output = run(root.path(), &["--output", "summary"]);
    let text = stdout(&output);

    assert!(output.status.success(), "stderr: {:?}", String::from_utf8_lossy(&output.stderr));
    assert!(text.contains("Fixture"));
    assert!(text.contains("Totals"));
    assert!(text.contains("Findings"));
}

#[test]
fn it_prints_a_parseable_report_as_json() {
    let root = fixture();

    let output = run(root.path(), &["--output", "json"]);
    assert!(output.status.success());

    let report: serde_json::Value =
        serde_json::from_str(stdout(&output).trim()).expect("the output is JSON");

    assert_eq!(report["project"]["name"], "Fixture");
    assert!(report["schema_version"].is_number());
    assert!(report["files"].is_array());
}

#[test]
fn it_writes_to_a_file_when_asked_and_prints_nothing() {
    let root = fixture();
    let target = root.path().join("report.json");

    let output = run(
        root.path(),
        &["--output", "json", "--write", &target.display().to_string()],
    );

    assert!(output.status.success());
    assert!(stdout(&output).trim().is_empty());

    let written = std::fs::read_to_string(&target).expect("the report was written");
    let report: serde_json::Value = serde_json::from_str(&written).expect("the file is JSON");
    assert_eq!(report["project"]["name"], "Fixture");
}

#[test]
fn hiding_tests_removes_them_from_the_summary() {
    let root = fixture();

    let shown = stdout(&run(root.path(), &["--output", "summary"]));
    let hidden = stdout(&run(root.path(), &["--output", "summary", "--hide-tests"]));

    assert!(shown.contains("tests/api.rs"));
    assert!(!hidden.contains("tests/api.rs"));
    assert!(hidden.contains("excluding tests"));
}

#[test]
fn the_dead_code_pass_can_be_turned_off() {
    let root = fixture();

    let with = stdout(&run(root.path(), &["--output", "json"]));
    let without = stdout(&run(root.path(), &["--output", "json", "--no-dead-code"]));

    assert!(with.contains("never_called"));

    let report: serde_json::Value = serde_json::from_str(without.trim()).expect("valid JSON");
    assert_eq!(
        report["dead_code"].as_array().map(Vec::len),
        Some(0),
        "--no-dead-code must empty the list, not merely hide it"
    );
}

#[test]
fn an_explicit_configuration_file_is_honored() {
    let root = fixture();
    let elsewhere = root.path().join("strict.toml");
    std::fs::write(
        &elsewhere,
        "[project]\nname = \"Strict\"\n\n[dependencies]\nenabled = false\n\n[thresholds]\nlarge_file_lines = 1\n",
    )
    .expect("the fixture is writable");

    let text = stdout(&run(
        root.path(),
        &["--output", "summary", "--config", &elsewhere.display().to_string()],
    ));

    assert!(text.contains("Strict"));
    assert!(text.contains("lines of code"));
}

#[test]
fn a_path_that_is_not_a_directory_fails_loudly() {
    let root = fixture();

    let output = run(&root.path().join("src/lib.rs"), &["--output", "summary"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("tinyanalyzer:"));
    assert!(stderr.contains("not a directory"));
}

#[test]
fn a_malformed_configuration_file_fails_loudly() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "tinyanalyzer.toml", "not [ valid toml");
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");

    let output = run(root.path(), &["--output", "summary"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot parse"));
}

#[test]
fn it_reports_a_version_and_a_help_page() {
    for flag in ["--version", "--help"] {
        let output = Command::new(BINARY)
            .arg(flag)
            .output()
            .expect("the binary was built for this test");

        assert!(output.status.success(), "{flag} must succeed");
        assert!(!output.stdout.is_empty(), "{flag} must print something");
    }
}
