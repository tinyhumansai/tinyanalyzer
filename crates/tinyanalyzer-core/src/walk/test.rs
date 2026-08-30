//! Unit tests for file discovery.
//!
//! These build real directory trees rather than mocking the filesystem: the
//! whole point of this module is that it agrees with `.gitignore` and with what
//! a developer sees, and neither is observable through a mock.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{SourceFile, discover};
use crate::config::ScanConfig;
use crate::error::Error;
use crate::loc::Language;
use std::path::Path;
use tempfile::TempDir;

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture is writable");
    }
    std::fs::write(path, contents).expect("the fixture is writable");
}

fn fixture() -> TempDir {
    let root = TempDir::new().expect("a temporary directory for the fixture");
    write(root.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");
    write(root.path(), "src/deep/inner.rs", "fn b() {}\n");
    write(root.path(), "tests/public_api.rs", "#[test]\nfn t() {}\n");
    write(root.path(), "target/debug/build.rs", "fn generated() {}\n");
    root
}

fn paths(files: &[SourceFile]) -> Vec<&str> {
    files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect()
}

#[test]
fn rejects_a_root_that_is_not_a_directory() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "a.rs", "fn a() {}\n");

    let error =
        discover(root.path().join("a.rs"), &ScanConfig::default()).expect_err("not a directory");

    assert!(matches!(error, Error::RootNotADirectory { .. }));
}

#[test]
fn finds_files_and_excludes_the_target_directory_by_default() {
    let root = fixture();

    let files = discover(root.path(), &ScanConfig::default()).expect("a walkable tree");

    assert_eq!(
        paths(&files),
        [
            "Cargo.toml",
            "src/deep/inner.rs",
            "src/lib.rs",
            "tests/public_api.rs"
        ]
    );
}

#[test]
fn results_are_sorted_so_two_runs_agree() {
    let root = fixture();
    let scan = ScanConfig::default();

    let first = discover(root.path(), &scan).expect("a walkable tree");
    let second = discover(root.path(), &scan).expect("a walkable tree");

    assert_eq!(paths(&first), paths(&second));
}

#[test]
fn an_include_list_narrows_the_walk() {
    let root = fixture();
    let scan = ScanConfig {
        include: vec!["src/**/*.rs".to_owned()],
        ..ScanConfig::default()
    };

    let files = discover(root.path(), &scan).expect("a walkable tree");

    assert_eq!(paths(&files), ["src/deep/inner.rs", "src/lib.rs"]);
}

#[test]
fn an_exclude_list_wins_over_an_include_list() {
    let root = fixture();
    let scan = ScanConfig {
        include: vec!["src/**/*.rs".to_owned()],
        exclude: vec!["src/deep/**".to_owned()],
        ..ScanConfig::default()
    };

    let files = discover(root.path(), &scan).expect("a walkable tree");

    assert_eq!(paths(&files), ["src/lib.rs"]);
}

#[test]
fn test_patterns_flag_paths_without_removing_them() {
    let root = fixture();

    let files = discover(root.path(), &ScanConfig::default()).expect("a walkable tree");
    let tests: Vec<&str> = files
        .iter()
        .filter(|file| file.is_test_path)
        .map(|file| file.relative_path.as_str())
        .collect();

    assert_eq!(tests, ["tests/public_api.rs"]);
}

#[test]
fn gitignored_files_are_skipped_when_the_walker_respects_ignore_files() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), ".gitignore", "generated/\n");
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");
    write(root.path(), "generated/big.rs", "fn generated() {}\n");

    let files = discover(root.path(), &ScanConfig::default()).expect("a walkable tree");

    assert_eq!(paths(&files), ["src/lib.rs"]);
}

#[test]
fn ignore_files_can_be_disabled() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), ".gitignore", "generated/\n");
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");
    write(root.path(), "generated/big.rs", "fn generated() {}\n");

    let scan = ScanConfig {
        respect_gitignore: false,
        ..ScanConfig::default()
    };
    let files = discover(root.path(), &scan).expect("a walkable tree");

    assert_eq!(paths(&files), ["generated/big.rs", "src/lib.rs"]);
}

#[test]
fn hidden_files_are_skipped_unless_asked_for() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");
    write(root.path(), ".hidden/secret.rs", "fn s() {}\n");

    let visible = discover(root.path(), &ScanConfig::default()).expect("a walkable tree");
    assert_eq!(paths(&visible), ["src/lib.rs"]);

    let scan = ScanConfig {
        include_hidden: true,
        respect_gitignore: false,
        ..ScanConfig::default()
    };
    let all = discover(root.path(), &scan).expect("a walkable tree");
    assert!(paths(&all).contains(&".hidden/secret.rs"));
}

#[test]
fn a_file_over_the_size_limit_is_counted_but_not_read() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "big.rs", &"// filler\n".repeat(100));

    let scan = ScanConfig {
        max_file_bytes: 10,
        ..ScanConfig::default()
    };
    let files = discover(root.path(), &scan).expect("a walkable tree");

    assert_eq!(files.len(), 1);
    assert!(files[0].text.is_none());
    assert!(files[0].bytes > 10);
}

#[test]
fn contents_and_language_come_back_with_the_file() {
    let root = TempDir::new().expect("a temporary directory");
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");

    let files = discover(root.path(), &ScanConfig::default()).expect("a walkable tree");

    assert_eq!(files[0].language, Language::Rust);
    assert_eq!(files[0].text.as_deref(), Some("pub fn a() {}\n"));
    assert!(files[0].absolute_path.is_absolute() || files[0].absolute_path.exists());
}

#[test]
fn a_file_reports_its_directory_and_name() {
    let root = fixture();

    let files = discover(root.path(), &ScanConfig::default()).expect("a walkable tree");
    let root_level = files
        .iter()
        .find(|file| file.relative_path == "Cargo.toml")
        .expect("the fixture writes a manifest");
    let nested = files
        .iter()
        .find(|file| file.relative_path == "src/deep/inner.rs")
        .expect("the fixture writes a nested file");

    assert_eq!(root_level.directory(), ".");
    assert_eq!(root_level.file_name(), "Cargo.toml");
    assert_eq!(nested.directory(), "src/deep");
    assert_eq!(nested.file_name(), "inner.rs");
}

#[test]
fn an_invalid_glob_is_reported_before_the_walk() {
    let root = fixture();
    let scan = ScanConfig {
        exclude: vec!["src/**{".to_owned()],
        ..ScanConfig::default()
    };

    let error = discover(root.path(), &scan).expect_err("a glob failure");

    assert!(matches!(error, Error::Glob { .. }));
}

#[test]
fn an_empty_directory_yields_no_files() {
    let root = TempDir::new().expect("a temporary directory");

    let files = discover(root.path(), &ScanConfig::default()).expect("a walkable tree");

    assert!(files.is_empty());
}
