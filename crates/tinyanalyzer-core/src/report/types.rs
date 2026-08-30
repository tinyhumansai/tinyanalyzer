//! The assembled report: everything one analysis produced.
//!
//! This is the crate's output type and the dashboard's input type. It is
//! serializable in full, so a report can be written to a file, compared against
//! a later one, or handed to something that is not this program at all.

use crate::config::Note;
use crate::dead_code::DeadCodeCandidate;
use crate::deps::DependencyReport;
use crate::findings::Finding;
use crate::loc::{Language, LineCounts};
use crate::rust_source::{ItemCounts, PerformanceSignals, RustFile};
use serde::{Deserialize, Serialize};

/// The version of the report format.
///
/// Bumped whenever a field is removed or changes meaning, so a consumer reading
/// a stored report can tell whether it still understands it. Adding a field
/// does not bump it.
pub const SCHEMA_VERSION: u32 = 1;

/// One complete analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The format version this report was written in.
    pub schema_version: u32,
    /// What was analyzed, and when.
    pub project: ProjectSummary,
    /// Whole-repository totals.
    pub totals: Totals,
    /// Every analyzed file, ranked heaviest first.
    pub files: Vec<FileMetrics>,
    /// Every directory holding at least one analyzed file.
    pub directories: Vec<DirectoryMetrics>,
    /// The language mix, largest first.
    pub languages: Vec<LanguageMetrics>,
    /// The resolved dependency graph.
    pub dependencies: DependencyReport,
    /// Items nothing appears to reference.
    pub dead_code: Vec<DeadCodeCandidate>,
    /// Everything worth doing something about, most severe first.
    pub findings: Vec<Finding>,
    /// Files that could not be parsed, and why.
    pub parse_failures: Vec<ParseFailureReport>,
}

impl Report {
    /// Serializes the report as indented JSON.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Serialize`] if the report cannot be encoded.
    pub fn to_json(&self) -> crate::Result<String> {
        serde_json::to_string_pretty(self).map_err(|source| crate::Error::Serialize { source })
    }

    /// Every file that is not test code.
    pub fn production_files(&self) -> impl Iterator<Item = &FileMetrics> {
        self.files.iter().filter(|file| !file.is_test)
    }

    /// Totals recomputed over production files only.
    ///
    /// This is what the dashboard's "hide tests" filter shows. It is computed
    /// rather than stored because the filter is a view, and storing both would
    /// mean two numbers that can disagree.
    #[must_use]
    pub fn production_totals(&self) -> Totals {
        let mut totals = Totals::default();

        for file in self.production_files() {
            totals.absorb(file);
            totals.lines = totals.lines.without(file.test_lines);
            if let Some(rust) = &file.rust {
                let test_functions = rust.functions.iter().filter(|function| function.is_test).count();
                totals.functions = totals.functions.saturating_sub(test_functions);
                totals.items.functions = totals.items.functions.saturating_sub(test_functions);
            }
        }

        totals.packages = self.totals.packages;
        totals.external_packages = self.totals.external_packages;
        totals.directories = self
            .directories
            .iter()
            .filter(|directory| !directory.is_test_only)
            .count();
        totals.test_files = 0;
        totals.test_lines = LineCounts::default();

        totals
    }
}

/// What was analyzed, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    /// Display name, from configuration or the root directory's name.
    pub name: String,
    /// Optional one-line description from configuration.
    pub description: Option<String>,
    /// Absolute path of the analysis root.
    pub root: String,
    /// Seconds since the Unix epoch at which the analysis ran.
    ///
    /// Zero when the system clock is unreadable, which is preferable to
    /// refusing to produce a report over a timestamp nobody reads.
    pub generated_at_unix: u64,
    /// The configuration file that was used, if any.
    pub config_file: Option<String>,
}

/// Whole-repository totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    /// Files analyzed.
    pub files: usize,
    /// Directories holding at least one analyzed file.
    pub directories: usize,
    /// Bytes on disk.
    pub bytes: u64,
    /// Lines, split by kind.
    pub lines: LineCounts,
    /// Files that are test code.
    pub test_files: usize,
    /// Lines belonging to test code.
    pub test_lines: LineCounts,
    /// Rust items defined.
    pub items: ItemCounts,
    /// Functions and methods measured.
    pub functions: usize,
    /// Cost signals summed across every Rust file.
    pub performance: PerformanceSignals,
    /// Packages in the resolved dependency graph.
    pub packages: usize,
    /// Packages that are not workspace members.
    pub external_packages: usize,
}

impl Totals {
    /// Folds one file's measurements into these totals.
    pub fn absorb(&mut self, file: &FileMetrics) {
        self.files = self.files.saturating_add(1);
        self.bytes = self.bytes.saturating_add(file.bytes);
        self.lines.add(file.lines);

        self.test_lines.add(file.test_lines);
        if file.is_test {
            self.test_files = self.test_files.saturating_add(1);
        }

        if let Some(rust) = &file.rust {
            self.items.add(rust.items);
            self.functions = self.functions.saturating_add(rust.functions.len());
            self.performance.add(rust.performance);
        }
    }
}

/// One analyzed file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileMetrics {
    /// Path relative to the analysis root, with forward slashes.
    pub path: String,
    /// The directory the file sits in, or `"."` for the root.
    pub directory: String,
    /// Language identified from the file name.
    pub language: Language,
    /// Size on disk.
    pub bytes: u64,
    /// Lines, split by kind.
    pub lines: LineCounts,
    /// Lines belonging to `#[test]` items or `#[cfg(test)]` scopes.
    #[serde(default)]
    pub test_lines: LineCounts,
    /// Whether the file is test code, by path or by contents.
    pub is_test: bool,
    /// The workspace member that owns the file, if any.
    pub crate_name: Option<String>,
    /// The parsed measurements, for a Rust file that parsed.
    pub rust: Option<RustFile>,
    /// How heavy this file is relative to the rest of the repository.
    ///
    /// See [`super::weight`] for what goes into it. The number is meaningless
    /// on its own and meaningful in a ranking, which is the only place it is
    /// used.
    pub weight: f64,
    /// Operator annotations from the configuration that match this path.
    pub notes: Vec<Note>,
}

/// One directory holding at least one analyzed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryMetrics {
    /// Path relative to the analysis root, or `"."` for the root.
    pub path: String,
    /// Files directly inside it.
    pub files: usize,
    /// Bytes of those files.
    pub bytes: u64,
    /// Lines of those files.
    pub lines: LineCounts,
    /// Whether every file in it is test code.
    pub is_test_only: bool,
}

/// One language's share of the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageMetrics {
    /// The language.
    pub language: Language,
    /// Files written in it.
    pub files: usize,
    /// Bytes of those files.
    pub bytes: u64,
    /// Lines of those files.
    pub lines: LineCounts,
}

/// A file the Rust parser refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseFailureReport {
    /// The file, relative to the analysis root.
    pub path: String,
    /// One-based line the parser gave up on.
    pub line: usize,
    /// What the parser objected to.
    pub message: String,
}
