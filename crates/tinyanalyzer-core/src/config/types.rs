//! The deserialized shape of `tinyanalyzer.toml`.
//!
//! Every field has a default, and every default is the value the analyzer would
//! use with no configuration file present at all. That is deliberate: a
//! repository should get a useful report before anybody writes a line of
//! configuration, and a configuration file should only ever be a set of
//! deviations from that baseline.

use serde::{Deserialize, Serialize};

/// The whole of `tinyanalyzer.toml`.
///
/// Load one with [`Config::load`](super::Config::load) rather than constructing
/// it by hand; [`Config::default`] is what an unconfigured repository gets, and
/// each section's own `Default` is the value that section documents.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Human-facing identity of the project under analysis.
    pub project: ProjectConfig,
    /// Which files the walker visits.
    pub scan: ScanConfig,
    /// The numbers that turn a measurement into a finding.
    pub thresholds: Thresholds,
    /// Dead-code detection settings.
    pub dead_code: DeadCodeConfig,
    /// Dependency analysis settings.
    pub dependencies: DependencyConfig,
    /// How the terminal dashboard behaves.
    pub ui: UiConfig,
    /// Operator annotations carried through to the dashboard.
    ///
    /// This is the "things to note" list: a path plus a sentence explaining why
    /// it looks the way it does. A file the team already knows is huge and has
    /// a note saying so reads very differently on a dashboard from one nobody
    /// has looked at.
    pub notes: Vec<Note>,
}

/// Human-facing identity of the project under analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    /// Display name for the dashboard header.
    ///
    /// Defaults to the workspace root's directory name when unset.
    pub name: Option<String>,
    /// One-line description shown beneath the name.
    pub description: Option<String>,
}

/// Which files the walker visits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanConfig {
    /// Globs a path must match to be analyzed, relative to the analysis root.
    ///
    /// Empty means "every file the other rules allow", which is the default.
    pub include: Vec<String>,
    /// Globs that remove a path from the analysis, applied after `include`.
    pub exclude: Vec<String>,
    /// Globs marking a path as test code.
    ///
    /// A path matching one of these is still analyzed, but is flagged so the
    /// dashboard's "hide tests" filter can take it out of every total. Rust
    /// files also get an AST-level test check, so this list only needs to cover
    /// what a path alone can tell you.
    pub test_patterns: Vec<String>,
    /// Whether `.gitignore`, `.ignore`, and friends are honored.
    ///
    /// On by default, so "analyze this repository" means the files a developer
    /// sees rather than every byte on disk.
    pub respect_gitignore: bool,
    /// Whether dotfiles and dot-directories are analyzed.
    pub include_hidden: bool,
    /// Whether the walker follows symbolic links.
    ///
    /// Off by default: a link pointing back up the tree turns a walk into a
    /// loop, and a link pointing outside the root silently widens the analysis.
    pub follow_symlinks: bool,
    /// Files larger than this many bytes are counted but not parsed.
    ///
    /// Generated and vendored sources reach sizes where parsing costs more than
    /// the result is worth; they still appear in the report with their line
    /// counts, just without item-level detail.
    pub max_file_bytes: u64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: vec![
                "target/**".to_owned(),
                "**/target/**".to_owned(),
                "vendor/**".to_owned(),
                "worktrees/**".to_owned(),
                "node_modules/**".to_owned(),
                ".git/**".to_owned(),
            ],
            test_patterns: vec![
                "**/tests/**".to_owned(),
                "**/test.rs".to_owned(),
                "**/tests.rs".to_owned(),
                "**/*_test.rs".to_owned(),
                "**/benches/**".to_owned(),
            ],
            respect_gitignore: true,
            include_hidden: false,
            follow_symlinks: false,
            max_file_bytes: 2_000_000,
        }
    }
}

/// The numbers that turn a measurement into a finding.
///
/// These exist so the rules in [`crate::findings`] have no magic constants: a
/// team that considers a 400-line file normal moves the number rather than
/// learning to ignore the warning.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    /// A source file at or above this many lines is worth splitting.
    pub large_file_lines: usize,
    /// A source file at or above this many lines is reported as urgent.
    pub huge_file_lines: usize,
    /// A function body at or above this many lines is worth splitting.
    pub long_function_lines: usize,
    /// A function at or above this cyclomatic complexity is worth simplifying.
    pub high_complexity: u32,
    /// A directory holding at or above this many files wants a submodule.
    pub large_directory_files: usize,
    /// A dependency pulling in at or above this many transitive crates is heavy.
    pub heavy_dependency_crates: usize,
    /// A file whose comment ratio falls below this is reported as underdocumented.
    ///
    /// Expressed as a fraction of code lines, so `0.05` means one comment line
    /// per twenty lines of code.
    pub min_comment_ratio: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            large_file_lines: 400,
            huge_file_lines: 800,
            long_function_lines: 60,
            high_complexity: 15,
            large_directory_files: 20,
            heavy_dependency_crates: 20,
            min_comment_ratio: 0.05,
        }
    }
}

/// Dead-code detection settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeadCodeConfig {
    /// Whether unreferenced items are looked for at all.
    pub enabled: bool,
    /// Item names never reported, however unreferenced they look.
    ///
    /// Entry points, ABI exports, and anything reached only by a macro belong
    /// here; the detector counts identifier occurrences and cannot see through
    /// a name that is assembled at expansion time.
    pub ignore: Vec<String>,
    /// Whether references from test code count as uses.
    ///
    /// Off by default, because an item used only by its own unit test is dead
    /// weight in the shipped binary — which is exactly what this tool is for.
    pub tests_count_as_uses: bool,
}

impl Default for DeadCodeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ignore: vec!["main".to_owned()],
            tests_count_as_uses: false,
        }
    }
}

/// Dependency analysis settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DependencyConfig {
    /// Whether `cargo metadata` is run at all.
    ///
    /// Turning this off makes the analysis pure filesystem work, which is what
    /// you want against a tree that does not resolve.
    pub enabled: bool,
    /// Whether development and build dependencies are included in the graph.
    pub include_dev: bool,
    /// Crate names never reported as unused, however unreferenced they look.
    ///
    /// A crate pulled in for its linker side effects, its `build.rs`, or a
    /// macro it exports has no `use` naming it.
    pub ignore_unused: Vec<String>,
}

impl Default for DependencyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_dev: true,
            ignore_unused: Vec::new(),
        }
    }
}

/// How the terminal dashboard behaves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Which view the dashboard opens on.
    pub start_view: StartView,
    /// Whether test code is hidden from every total when the dashboard opens.
    ///
    /// The filter is toggleable at runtime; this is only its initial state.
    pub hide_tests: bool,
    /// How many rows each ranked table shows before it scrolls.
    pub table_rows: usize,
    /// Whether the dashboard draws with Unicode box-drawing characters.
    ///
    /// Turning this off falls back to ASCII, which is what you want over a
    /// connection or in a terminal whose font has no box-drawing glyphs.
    pub unicode: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            start_view: StartView::default(),
            hide_tests: false,
            table_rows: 20,
            unicode: true,
        }
    }
}

/// The dashboard view shown on startup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartView {
    /// Totals, language mix, and the headline findings.
    #[default]
    Overview,
    /// Files ranked by weight.
    Files,
    /// The dependency graph and the heaviest crates in it.
    Dependencies,
    /// Unreferenced items.
    DeadCode,
    /// Every finding, ranked by severity.
    Findings,
}

/// An operator annotation attached to a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// Path the note is about, relative to the analysis root.
    ///
    /// Matched as a glob, so a note can cover a directory.
    pub path: String,
    /// What the reader should know about it.
    pub note: String,
    /// Optional severity hint, surfaced as a badge on the dashboard.
    #[serde(default)]
    pub level: NoteLevel,
}

/// How prominently a [`Note`] is displayed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteLevel {
    /// Background context. The default.
    #[default]
    Info,
    /// Something the team has agreed to fix.
    Warning,
    /// Something actively hurting the project.
    Critical,
}

impl NoteLevel {
    /// The word used for this level on the dashboard.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}
