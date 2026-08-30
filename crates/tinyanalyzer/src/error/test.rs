//! Unit tests for the crate-wide error type.

use super::{Error, Result};
use std::error::Error as _;
use std::io::{Error as IoError, ErrorKind};
use std::path::PathBuf;

#[test]
fn an_analysis_failure_renders_as_the_engine_wrote_it() {
    let inner = tinyanalyzer_core::Error::RootNotADirectory {
        path: PathBuf::from("/nope"),
    };
    let expected = inner.to_string();
    let error = Error::from(inner);

    assert_eq!(error.to_string(), expected);
    assert!(matches!(error, Error::Analysis(_)));
}

#[test]
fn a_terminal_failure_says_so_and_keeps_its_source() {
    let error = Error::Terminal {
        source: IoError::new(ErrorKind::Other, "not a tty"),
    };

    assert_eq!(error.to_string(), "cannot control the terminal: not a tty");
    assert!(error.source().is_some());
}

#[test]
fn a_write_failure_names_the_path() {
    let error = Error::Write {
        path: PathBuf::from("report.json"),
        source: IoError::new(ErrorKind::PermissionDenied, "denied"),
    };

    assert_eq!(error.to_string(), "cannot write report.json: denied");
}

#[test]
fn the_result_alias_carries_the_crate_error() {
    let ok: Result<u8> = Ok(1);

    assert_eq!(ok.unwrap(), 1);
}
