//! The analysis engine behind `tinyanalyzer`.
//!
//! Point this crate at a Rust repository and it produces a [`Report`]: every
//! file with its line counts and parsed structure, every directory and language
//! aggregated, the resolved dependency graph with the real cost of each direct
//! dependency, the items nothing references, and a ranked list of things worth
//! fixing with a specific remedy attached to each one.
//!
//! # What this crate deliberately does not hold
//!
//! No terminal, no rendering, no event loop, no I/O beyond reading the files it
//! was asked about and running `cargo metadata`. The dashboard lives in the
//! `tinyanalyzer` crate, which depends on this one and re-exports it.
//!
//! That direction is load-bearing. Analysis is the part worth embedding in
//! something else — a CI check, an editor plugin, a report diffed between two
//! commits — and none of those want a terminal UI linked into them. The split
//! is asserted in CI rather than merely documented, because a dependency on a
//! renderer arrives transitively through a feature somebody enabled one crate
//! away and is invisible in the diff that introduces it.
//!
//! # Pipeline
//!
//! [`report::analyze`] runs the modules below in order. Each is usable on its
//! own, and each is documented with what it measures and what it deliberately
//! approximates.
//!
//! | Module | What it answers |
//! |---|---|
//! | [`config`] | What did the operator ask for? |
//! | [`walk`] | Which files are in scope? |
//! | [`loc`] | How much of each file is code, comment, and blank? |
//! | [`rust_source`] | What does each Rust file define, and how tangled is it? |
//! | [`deps`] | What does the dependency graph actually cost? |
//! | [`dead_code`] | What does nothing reference? |
//! | [`findings`] | What should somebody do about all this? |
//! | [`report`] | All of it, joined and ranked. |
//!
//! # Example
//!
//! ```no_run
//! use tinyanalyzer_core::{Report, analyze};
//!
//! let report: Report = analyze(".")?;
//!
//! println!("{} files, {} lines", report.totals.files, report.totals.lines.code);
//!
//! for finding in report.findings.iter().take(5) {
//!     println!("[{}] {}", finding.severity.label(), finding.title);
//!     println!("    {}", finding.suggestion);
//! }
//! # Ok::<(), tinyanalyzer_core::Error>(())
//! ```

pub mod config;
pub mod dead_code;
pub mod deps;
pub mod error;
pub mod findings;
pub mod loc;
pub mod report;
pub mod rust_source;
pub mod walk;

// The public surface, centralized here so a consumer has one predictable place
// to import from and every path through this crate resolves to the same
// definitions.
pub use config::{
    CONFIG_FILE_NAME, CONFIG_FILE_NAME_ALT, Config, DeadCodeConfig, DependencyConfig, Note,
    NoteLevel, ProjectConfig, ScanConfig, StartView, Thresholds, UiConfig,
};
pub use dead_code::{Confidence, DeadCodeCandidate};
pub use deps::{
    DependencyEdge, DependencyKind, DependencyReport, DuplicateVersions, PackageNode,
    UnusedDependency,
};
pub use error::{Error, Result};
pub use findings::{Finding, Location, Rule, Severity};
pub use loc::{Language, LineCounts, count_lines};
pub use report::{
    DirectoryMetrics, FileMetrics, LanguageMetrics, ParseFailureReport, ProjectSummary, Report,
    SCHEMA_VERSION, Totals, analyze, analyze_with,
};
pub use rust_source::{
    Definition, DefinitionKind, Function, ItemCounts, PerformanceSignals, RustFile,
};
pub use walk::SourceFile;
