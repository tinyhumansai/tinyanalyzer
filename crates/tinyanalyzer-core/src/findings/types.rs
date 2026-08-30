//! What a rule produces.

use serde::{Deserialize, Serialize};

/// One thing worth doing something about.
///
/// Every finding names the rule that produced it, says what was measured, and
/// says what to do. A finding that only says something is wrong makes the
/// reader do the work twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// The rule that produced this finding.
    pub rule: Rule,
    /// How much it matters.
    pub severity: Severity,
    /// One line naming the problem.
    pub title: String,
    /// What was measured, in a sentence, with the numbers in it.
    pub detail: String,
    /// What to do about it.
    pub suggestion: String,
    /// Where to look, when there is one place.
    pub location: Option<Location>,
    /// The measurement behind the finding, for ranking within a rule.
    pub metric: f64,
}

/// Where a finding points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// File, relative to the analysis root.
    pub file: String,
    /// One-based line, when the finding is about a specific one.
    pub line: Option<usize>,
}

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Actively costing the project, now.
    Critical,
    /// Worth scheduling.
    High,
    /// Worth knowing.
    Medium,
    /// Background information.
    Low,
}

impl Severity {
    /// The word used for this level on the dashboard.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// The stable identifier of a rule.
///
/// These are part of the report format: a rule name appears in stored reports
/// and in any configuration that suppresses one, so renaming a variant is a
/// breaking change to the schema, not a refactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Rule {
    /// A source file well past the size a reader can hold in their head.
    HugeFile,
    /// A source file past the configured size.
    LargeFile,
    /// A function body past the configured length.
    LongFunction,
    /// A function with more paths through it than the configured limit.
    ComplexFunction,
    /// Block nesting deep enough to be worth flattening.
    DeepNesting,
    /// An allocation inside a loop body.
    AllocationInLoop,
    /// A loop directly inside another loop.
    NestedLoop,
    /// A direct dependency pulling in a large exclusive subtree.
    HeavyDependency,
    /// A crate resolved at more than one version.
    DuplicateDependency,
    /// A declared dependency no source file names.
    UnusedDependency,
    /// Items nothing in the workspace references.
    DeadCode,
    /// A directory holding more files than it can explain.
    LargeDirectory,
    /// A file with far fewer comments than code.
    Underdocumented,
    /// A panic path in code that is not a test.
    PanicPath,
    /// Unfinished-work markers left in comments.
    UnfinishedWork,
    /// A file the Rust parser refused.
    ParseFailure,
}

impl Rule {
    /// The stable string identifier, matching the serialized form.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::HugeFile => "huge_file",
            Self::LargeFile => "large_file",
            Self::LongFunction => "long_function",
            Self::ComplexFunction => "complex_function",
            Self::DeepNesting => "deep_nesting",
            Self::AllocationInLoop => "allocation_in_loop",
            Self::NestedLoop => "nested_loop",
            Self::HeavyDependency => "heavy_dependency",
            Self::DuplicateDependency => "duplicate_dependency",
            Self::UnusedDependency => "unused_dependency",
            Self::DeadCode => "dead_code",
            Self::LargeDirectory => "large_directory",
            Self::Underdocumented => "underdocumented",
            Self::PanicPath => "panic_path",
            Self::UnfinishedWork => "unfinished_work",
            Self::ParseFailure => "parse_failure",
        }
    }

    /// What the rule looks for, in one sentence.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::HugeFile => "files far past the size a reader can hold in their head",
            Self::LargeFile => "files past the configured size",
            Self::LongFunction => "functions past the configured length",
            Self::ComplexFunction => "functions with more branches than the configured limit",
            Self::DeepNesting => "blocks nested deeply enough to be worth flattening",
            Self::AllocationInLoop => "allocations that happen once per iteration",
            Self::NestedLoop => "loops directly inside other loops",
            Self::HeavyDependency => "dependencies pulling in a large exclusive subtree",
            Self::DuplicateDependency => "crates compiled twice at two versions",
            Self::UnusedDependency => "declared dependencies no source file names",
            Self::DeadCode => "items nothing in the workspace references",
            Self::LargeDirectory => "directories holding more files than they can explain",
            Self::Underdocumented => "files with far fewer comments than code",
            Self::PanicPath => "panic paths outside test code",
            Self::UnfinishedWork => "unfinished-work markers left in comments",
            Self::ParseFailure => "files the Rust parser refused",
        }
    }
}
