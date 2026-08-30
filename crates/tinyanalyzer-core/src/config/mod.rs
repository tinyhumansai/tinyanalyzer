//! Reading `tinyanalyzer.toml`.
//!
//! The configuration file is optional. [`Config::load`] returns
//! [`Config::default`] when no file is present, so the analyzer produces a
//! useful report against a repository that has never heard of it, and a
//! configuration file only ever records deviations from that baseline.
//!
//! Everything the file can say lives in [`types`]; this module is the loading,
//! the file-name policy, and the note lookup the report assembly uses.
//!
//! # Example
//!
//! ```
//! use tinyanalyzer_core::Config;
//!
//! let config: Config = toml::from_str(
//!     r#"
//!     [thresholds]
//!     large_file_lines = 250
//!
//!     [[notes]]
//!     path = "src/parser/**"
//!     note = "hand-written parser; long functions are deliberate"
//!     level = "info"
//!     "#,
//! )?;
//!
//! assert_eq!(config.thresholds.large_file_lines, 250);
//! assert_eq!(config.notes.len(), 1);
//! # Ok::<(), toml::de::Error>(())
//! ```

mod types;

pub use types::{
    Config, DeadCodeConfig, DependencyConfig, Note, NoteLevel, ProjectConfig, ScanConfig,
    StartView, Thresholds, UiConfig,
};

use crate::error::{Error, Result};
use globset::{Glob, GlobSetBuilder};
use std::path::{Path, PathBuf};

/// The configuration file name this tool looks for.
pub const CONFIG_FILE_NAME: &str = "tinyanalyzer.toml";

/// An accepted alternative spelling of [`CONFIG_FILE_NAME`].
///
/// Both spellings are in use in the wild and picking one to reject would only
/// produce a silently unconfigured analysis, which is the worst outcome
/// available: the report still renders, just with the wrong numbers.
pub const CONFIG_FILE_NAME_ALT: &str = "tiny-analyzer.toml";

impl Config {
    /// Loads the configuration for the repository rooted at `root`.
    ///
    /// Looks for [`CONFIG_FILE_NAME`] and then [`CONFIG_FILE_NAME_ALT`] in
    /// `root`. Returns [`Config::default`] when neither exists — an
    /// unconfigured repository is the normal case, not an error.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if a configuration file exists but cannot be read,
    /// and [`Error::Config`] if it can be read but is not valid TOML or does
    /// not match the schema.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();

        match Self::locate(root) {
            Some(path) => Self::from_file(path),
            None => Ok(Self::default()),
        }
    }

    /// Returns the configuration file `root` would be read from, if any.
    ///
    /// Useful for telling a user which file an analysis actually used, which is
    /// otherwise invisible when two spellings are accepted.
    #[must_use]
    pub fn locate(root: impl AsRef<Path>) -> Option<PathBuf> {
        let root = root.as_ref();

        [CONFIG_FILE_NAME, CONFIG_FILE_NAME_ALT]
            .into_iter()
            .map(|name| root.join(name))
            .find(|candidate| candidate.is_file())
    }

    /// Reads and parses a configuration file at an explicit path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be read and [`Error::Config`]
    /// if its contents do not parse.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| Error::io(path, source))?;

        toml::from_str(&text).map_err(|source| Error::Config {
            path: path.to_path_buf(),
            message: source.to_string(),
        })
    }

    /// Returns every note whose glob matches `path`.
    ///
    /// `path` is interpreted relative to the analysis root, matching the way
    /// note paths are written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Glob`] if a note's path is not a valid glob.
    pub fn notes_for(&self, path: &str) -> Result<Vec<&Note>> {
        let mut matched = Vec::new();

        for note in &self.notes {
            if compile_glob(&note.path)?.compile_matcher().is_match(path) {
                matched.push(note);
            }
        }

        Ok(matched)
    }

    /// Returns the display name for the project rooted at `root`.
    ///
    /// Falls back to the root directory's own name, then to the literal
    /// `unnamed project` for a root with no final component — a bare `/`, in
    /// practice.
    #[must_use]
    pub fn display_name(&self, root: &Path) -> String {
        if let Some(name) = &self.project.name {
            return name.clone();
        }

        root.file_name()
            .map_or_else(|| "unnamed project".to_owned(), |name| {
                name.to_string_lossy().into_owned()
            })
    }
}

/// Compiles one glob pattern, mapping the glob crate's error into ours.
///
/// # Errors
///
/// Returns [`Error::Glob`] when `pattern` is not a valid glob.
pub(crate) fn compile_glob(pattern: &str) -> Result<Glob> {
    Glob::new(pattern).map_err(|source| Error::Glob {
        pattern: pattern.to_owned(),
        message: source.to_string(),
    })
}

/// Compiles a list of patterns into a single matcher.
///
/// Returns `None` for an empty list so a caller can distinguish "matches
/// nothing" from "no opinion", which are opposite answers for an include list.
///
/// # Errors
///
/// Returns [`Error::Glob`] when any pattern is not a valid glob.
pub(crate) fn compile_glob_set(patterns: &[String]) -> Result<Option<globset::GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(compile_glob(pattern)?);
    }

    builder
        .build()
        .map(Some)
        .map_err(|source| Error::Glob {
            pattern: patterns.join(", "),
            message: source.to_string(),
        })
}

#[cfg(test)]
mod test;
