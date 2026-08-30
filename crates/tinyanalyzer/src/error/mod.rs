//! Crate-wide error and result types.
//!
//! The analysis engine has its own error type; this one wraps it and adds the
//! failures that only exist once there is a terminal and a command line in the
//! picture. A caller matching on a variant should be able to tell which half of
//! the program failed without reading the message.

use std::path::PathBuf;

/// Errors returned by this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The analysis itself failed.
    #[error(transparent)]
    Analysis(#[from] tinyanalyzer_core::Error),

    /// The terminal could not be put into, or taken out of, raw mode.
    ///
    /// A failure here is worth its own variant because it is the one failure
    /// that can leave the user's terminal unusable, and the caller's remedy —
    /// tell them to run `reset` — is different from every other error's.
    #[error("cannot control the terminal: {source}")]
    Terminal {
        /// The underlying operating-system error.
        #[source]
        source: std::io::Error,
    },

    /// A file could not be written.
    #[error("cannot write {path}: {source}")]
    Write {
        /// The path that could not be written.
        path: PathBuf,
        /// The underlying operating-system error.
        #[source]
        source: std::io::Error,
    },
}

/// The crate's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test;
