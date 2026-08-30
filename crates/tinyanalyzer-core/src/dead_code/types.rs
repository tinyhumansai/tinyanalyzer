//! What dead-code analysis reports and how sure it is.

use crate::rust_source::{DefinitionKind, RustFile};
use serde::{Deserialize, Serialize};

/// One file's contribution to the workspace-wide symbol census.
#[derive(Debug, Clone, Copy)]
pub struct DeadCodeInput<'a> {
    /// The file's path relative to the analysis root.
    pub path: &'a str,
    /// The parsed file.
    pub rust: &'a RustFile,
    /// Whether the file is test code.
    pub is_test_file: bool,
}

/// An item nothing in the workspace appears to reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadCodeCandidate {
    /// The item's name.
    pub name: String,
    /// What kind of item it is.
    pub kind: DefinitionKind,
    /// The file that defines it, relative to the analysis root.
    pub file: String,
    /// One-based line of the definition.
    pub line: usize,
    /// Whether the item is `pub`.
    pub is_public: bool,
    /// Whether the definition is test-only.
    pub is_test: bool,
    /// How much weight to put on this candidate.
    pub confidence: Confidence,
    /// Why the analyzer reached that confidence, in one sentence.
    pub reason: String,
}

/// How much weight to put on a dead-code candidate.
///
/// The detector counts identifier occurrences across the workspace. That is a
/// strong signal for a private item, whose every possible caller is in the same
/// crate, and a much weaker one for a `pub` item in a library, whose callers may
/// not be in this repository at all. Reporting both at the same volume would
/// train a reader to ignore the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Every possible caller was in scope and none of them names it.
    High,
    /// The item is public, so a caller outside this workspace is possible.
    Medium,
}

impl Confidence {
    /// The word used for this level on the dashboard.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
        }
    }
}
