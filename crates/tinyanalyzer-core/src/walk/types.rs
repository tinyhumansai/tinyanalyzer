//! What the walker produces and the filter that decides what it visits.

use crate::loc::Language;
use std::path::PathBuf;

/// One file the analyzer decided to look at.
///
/// The walker reads contents eagerly because every downstream analyzer needs
/// them and reading twice is the dominant cost on a large tree. A file above
/// the configured size limit is still reported — with its byte count and no
/// text — so it appears in the totals it belongs in rather than vanishing.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Path relative to the analysis root, with forward slashes.
    ///
    /// Normalized so a report generated on Windows and one generated on Linux
    /// name the same file the same way, which matters the moment two of them
    /// are compared.
    pub relative_path: String,
    /// Absolute path on disk.
    pub absolute_path: PathBuf,
    /// Language identified from the file name.
    pub language: Language,
    /// Size on disk.
    pub bytes: u64,
    /// The file's contents, if it was small enough and is valid UTF-8.
    ///
    /// `None` means "counted but not read": too large, or not text.
    pub text: Option<String>,
    /// Whether the path alone marks this as test code.
    ///
    /// Rust files get a second, AST-level check in [`crate::rust_source`]; this
    /// flag only reflects what the path says.
    pub is_test_path: bool,
}

impl SourceFile {
    /// The directory this file sits in, relative to the analysis root.
    ///
    /// Returns `"."` for a file at the root, so every file has a parent to be
    /// grouped under and the directory table has no missing rows.
    #[must_use]
    pub fn directory(&self) -> &str {
        match self.relative_path.rsplit_once('/') {
            Some((parent, _)) => parent,
            None => ".",
        }
    }

    /// The file's own name, without its directory.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.relative_path
            .rsplit_once('/')
            .map_or(self.relative_path.as_str(), |(_, name)| name)
    }
}
