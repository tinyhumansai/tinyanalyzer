//! Unit tests for the crate-wide error type.
//!
//! These pin the rendered messages, because they are what a user sees on the
//! terminal when an analysis fails, and the `#[source]` chain, because that is
//! what a caller walks to find the real cause.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Error, Result};
use std::error::Error as _;
use std::io::{Error as IoError, ErrorKind};
use std::path::PathBuf;

#[test]
fn renders_a_missing_root_with_its_path() {
    let error = Error::RootNotADirectory {
        path: PathBuf::from("/nope"),
    };

    assert_eq!(error.to_string(), "analysis root is not a directory: /nope");
}

#[test]
fn the_io_constructor_attaches_the_path_and_keeps_the_source() {
    let error = Error::io("src/lib.rs", IoError::new(ErrorKind::NotFound, "missing"));

    assert_eq!(error.to_string(), "cannot read src/lib.rs: missing");
    assert!(error.source().is_some());
}

#[test]
fn renders_a_configuration_failure_with_the_parser_message() {
    let error = Error::Config {
        path: PathBuf::from("tinyanalyzer.toml"),
        message: "expected a table".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "cannot parse tinyanalyzer.toml: expected a table"
    );
}

#[test]
fn renders_a_glob_failure_with_the_offending_pattern() {
    let error = Error::Glob {
        pattern: "src/**{".to_owned(),
        message: "unclosed alternate group".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "invalid glob pattern \"src/**{\": unclosed alternate group"
    );
}

#[test]
fn renders_a_walk_failure_with_the_root() {
    let error = Error::Walk {
        root: PathBuf::from("crates"),
        message: "permission denied".to_owned(),
    };

    assert_eq!(error.to_string(), "cannot walk crates: permission denied");
}

#[test]
fn renders_a_cargo_metadata_failure_with_the_root() {
    let error = Error::CargoMetadata {
        root: PathBuf::from("."),
        message: "no Cargo.toml".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "cannot read cargo metadata for .: no Cargo.toml"
    );
}

#[test]
fn renders_a_serialization_failure_and_keeps_its_source() {
    let source = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let error = Error::Serialize { source };

    assert!(error.to_string().starts_with("cannot serialize report: "));
    assert!(error.source().is_some());
}

#[test]
fn the_result_alias_carries_the_crate_error() {
    let ok: Result<u8> = Ok(1);
    let failed: Result<u8> = Err(Error::RootNotADirectory {
        path: PathBuf::from("/nope"),
    });

    assert!(matches!(ok, Ok(1)));
    assert!(matches!(failed, Err(Error::RootNotADirectory { .. })));
}
