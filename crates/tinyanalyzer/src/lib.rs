//! Analyze a Rust codebase and explore it in a terminal dashboard.
//!
//! This crate is the shipped program: a command line, a non-interactive text
//! and JSON report, and an interactive terminal dashboard over the analysis
//! that [`tinyanalyzer_core`] produces.
//!
//! # Layout
//!
//! This is the interface half of a two-crate workspace:
//!
//! - [`tinyanalyzer_core`] — the analysis engine. File metrics, Rust item and
//!   complexity analysis, the dependency graph, dead-code detection, and the
//!   rules that turn all of it into advice. No terminal, no rendering, no event
//!   loop. Something embedding the analyzer depends on that crate alone.
//! - `tinyanalyzer` — this crate. The command line, the renderers, and the
//!   `tinyanalyzer` binary.
//!
//! Within this crate:
//!
//! - [`cli`] is the command line and the configuration it produces.
//! - [`summary`] renders a report as text, for a pipe or a pull request.
//! - [`dashboard`] is the interactive view: a state machine, a renderer, and the
//!   event loop that connects them.
//! - [`error`] holds the crate-wide [`Error`] and the [`Result`] alias every
//!   fallible public function returns.
//!
//! Every public item of the engine is re-exported from here, so
//! `tinyanalyzer::Report` and `tinyanalyzer_core::Report` are the *same type*
//! rather than structural twins.
//!
//! # Example
//!
//! ```no_run
//! use tinyanalyzer::{Report, analyze, summary};
//!
//! let report: Report = analyze(".")?;
//!
//! print!("{}", summary::render(&report, true));
//! # Ok::<(), tinyanalyzer::Error>(())
//! ```

pub mod cli;
pub mod dashboard;
pub mod error;
pub mod summary;

pub use cli::{Cli, Output};
pub use dashboard::{Action, Dashboard, View};
pub use error::{Error, Result};

// The analysis engine, re-exported by module rather than by item so every path
// through this crate resolves to the same definitions the engine publishes. A
// consumer may depend on `tinyanalyzer-core` directly and get exactly these
// types; nothing here redefines them.
pub use tinyanalyzer_core;
pub use tinyanalyzer_core::{
    CONFIG_FILE_NAME, CONFIG_FILE_NAME_ALT, Confidence, Config, DeadCodeCandidate,
    DependencyReport, DirectoryMetrics, FileMetrics, Finding, Language, LineCounts, PackageNode,
    Report, Rule, Severity, StartView, Thresholds, Totals, analyze, analyze_with,
};
