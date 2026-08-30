//! Crate-wide error and result types.
//!
//! Every fallible public function in this crate returns [`Result`], and every
//! failure mode is a distinct [`Error`] variant. Add a variant rather than
//! encoding new context into an existing message: callers match on variants,
//! and message text is not a stable API.
//!
//! Variants carry the data a caller needs to react — the path that could not be
//! read, the manifest cargo refused — keep their `#[error]` message lowercase
//! and free of trailing punctuation, and are documented so the rendered
//! rustdoc explains when each one occurs.

use std::path::PathBuf;

/// Errors returned by this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The analysis root does not exist, or is not a directory.
    #[error("analysis root is not a directory: {path}")]
    RootNotADirectory {
        /// The path that was given as the analysis root.
        path: PathBuf,
    },

    /// A file or directory could not be read from disk.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The path whose read failed.
        path: PathBuf,
        /// The underlying operating-system error.
        #[source]
        source: std::io::Error,
    },

    /// `tinyanalyzer.toml` was found but is not valid TOML, or does not match
    /// the configuration schema.
    #[error("cannot parse {path}: {message}")]
    Config {
        /// The configuration file that failed to parse.
        path: PathBuf,
        /// The parser's description of what is wrong.
        message: String,
    },

    /// An include or exclude entry in the configuration is not a valid glob.
    #[error("invalid glob pattern {pattern:?}: {message}")]
    Glob {
        /// The pattern as written in the configuration.
        pattern: String,
        /// Why the glob compiler rejected it.
        message: String,
    },

    /// Directory traversal failed part-way through.
    #[error("cannot walk {root}: {message}")]
    Walk {
        /// The directory being traversed.
        root: PathBuf,
        /// The traversal error.
        message: String,
    },

    /// `cargo metadata` could not be run, or returned something unusable.
    ///
    /// The dependency graph is cargo's own resolution rather than a re-parse of
    /// manifests, so a workspace that does not resolve has no dependency graph
    /// to report.
    #[error("cannot read cargo metadata for {root}: {message}")]
    CargoMetadata {
        /// The workspace root `cargo metadata` was run against.
        root: PathBuf,
        /// What cargo reported.
        message: String,
    },

    /// A report could not be encoded as JSON.
    #[error("cannot serialize report: {source}")]
    Serialize {
        /// The underlying `serde_json` failure.
        #[source]
        source: serde_json::Error,
    },
}

impl Error {
    /// Builds an [`Error::Io`] for `path` from a standard I/O error.
    ///
    /// Reading a file is the single most repeated fallible operation in this
    /// crate, and the path is the only context worth attaching, so it gets a
    /// constructor rather than a `map_err` closure at every call site.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// The crate's standard result type.
///
/// Use this alias in public signatures instead of spelling out
/// `std::result::Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test;
