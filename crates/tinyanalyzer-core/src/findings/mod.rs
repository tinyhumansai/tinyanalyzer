//! Turning measurements into advice.
//!
//! Everything up to this point counts things. This module is the part that has
//! an opinion: it reads the counts and says what to do about them. Each rule is
//! a small function that takes the measurements it needs and emits zero or more
//! [`Finding`]s, and every rule obeys the same three constraints:
//!
//! 1. **Every threshold comes from [`Thresholds`].** A rule with a constant in
//!    it is a rule a team cannot disagree with, and they will simply stop
//!    reading the output instead.
//! 2. **Every finding says what was measured.** "This file is large" is not
//!    actionable; "1,240 lines, 8 items, longest function 210 lines" is.
//! 3. **Every finding says what to do.** A reader who has to work out the remedy
//!    from the diagnosis is doing the work twice.
//!
//! Findings are ranked by severity and then by the measurement behind them, so
//! the top of the list is the thing most worth fixing rather than the first
//! thing the walker happened to see.

mod types;

pub use types::{Finding, Location, Rule, Severity};

use crate::config::Thresholds;
use crate::dead_code::{Confidence, DeadCodeCandidate};
use crate::deps::DependencyReport;
use crate::report::{DirectoryMetrics, FileMetrics, ParseFailureReport};

/// Everything the rules read.
///
/// Grouped into one struct rather than passed as a handful of slices so that
/// adding a rule that needs a new input does not change every call site.
#[derive(Debug, Clone, Copy)]
pub struct FindingInputs<'a> {
    /// Every analyzed file.
    pub files: &'a [FileMetrics],
    /// Every directory holding an analyzed file.
    pub directories: &'a [DirectoryMetrics],
    /// The resolved dependency graph.
    pub dependencies: &'a DependencyReport,
    /// Items nothing appears to reference.
    pub dead_code: &'a [DeadCodeCandidate],
    /// Files the Rust parser refused.
    pub parse_failures: &'a [ParseFailureReport],
}

/// Runs every rule and returns the findings, most severe first.
#[must_use]
pub fn analyze(inputs: FindingInputs<'_>, thresholds: &Thresholds) -> Vec<Finding> {
    let mut findings = Vec::new();

    for file in inputs.files {
        file_size(file, thresholds, &mut findings);
        documentation(file, thresholds, &mut findings);
        rust_rules(file, thresholds, &mut findings);
    }

    directories(inputs.directories, thresholds, &mut findings);
    dependencies(inputs.dependencies, thresholds, &mut findings);
    dead_code(inputs.dead_code, &mut findings);
    parse_failures(inputs.parse_failures, &mut findings);

    findings.sort_by(|left, right| {
        left.severity.cmp(&right.severity).then_with(|| {
            right
                .metric
                .partial_cmp(&left.metric)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    findings
}

/// Builds a finding, keeping the construction noise out of the rules.
fn finding(
    rule: Rule,
    severity: Severity,
    title: String,
    detail: String,
    suggestion: String,
    location: Option<Location>,
    metric: f64,
) -> Finding {
    Finding {
        rule,
        severity,
        title,
        detail,
        suggestion,
        location,
        metric,
    }
}

/// Points at a whole file.
fn at_file(path: &str) -> Option<Location> {
    Some(Location {
        file: path.to_owned(),
        line: None,
    })
}

/// Points at one line of a file.
fn at_line(path: &str, line: usize) -> Option<Location> {
    Some(Location {
        file: path.to_owned(),
        line: Some(line),
    })
}

/// Widens a count for use as a ranking metric.
///
/// Every count in this crate is a line number, an item count, or a package
/// count — all far below the range where `f64` loses integer precision.
#[allow(clippy::cast_precision_loss)]
fn metric(count: usize) -> f64 {
    count as f64
}

/// Flags files past the configured sizes.
fn file_size(file: &FileMetrics, thresholds: &Thresholds, out: &mut Vec<Finding>) {
    let lines = file.lines.code;

    let (rule, severity) = if lines >= thresholds.huge_file_lines {
        (Rule::HugeFile, Severity::High)
    } else if lines >= thresholds.large_file_lines {
        (Rule::LargeFile, Severity::Medium)
    } else {
        return;
    };

    let items = file.rust.as_ref().map_or(0, |rust| rust.items.total());
    let functions = file.rust.as_ref().map_or(0, |rust| rust.functions.len());

    out.push(finding(
        rule,
        severity,
        format!("{} is {lines} lines of code", file.path),
        format!(
            "{lines} lines of code across {items} items and {functions} functions, against a threshold of {}.",
            if rule == Rule::HugeFile {
                thresholds.huge_file_lines
            } else {
                thresholds.large_file_lines
            }
        ),
        "Split it along the seams already in it: each group of functions that share state or a concern becomes a module directory with its own `mod.rs`, `types.rs`, and `test.rs`.".to_owned(),
        at_file(&file.path),
        metric(lines),
    ));
}

/// Flags files whose comment ratio is far below the configured floor.
fn documentation(file: &FileMetrics, thresholds: &Thresholds, out: &mut Vec<Finding>) {
    // A language with no comment syntax cannot be underdocumented, and a small
    // file has too little signal to say anything about.
    if file.is_test || !file.language.has_comment_syntax() || file.lines.code < 50 {
        return;
    }

    let ratio = file.lines.comment_ratio();
    if ratio >= thresholds.min_comment_ratio {
        return;
    }

    out.push(finding(
        Rule::Underdocumented,
        Severity::Low,
        format!("{} carries almost no explanation", file.path),
        format!(
            "{} comment lines against {} lines of code, a ratio of {ratio:.3} where {:.3} is the floor.",
            file.lines.comment, file.lines.code, thresholds.min_comment_ratio
        ),
        "Add a module-level `//!` saying what the file is for and why it exists, then a rustdoc line on each public item explaining when a caller would reach for it.".to_owned(),
        at_file(&file.path),
        metric(file.lines.code),
    ));
}

/// Runs every rule that needs a parsed Rust file.
fn rust_rules(file: &FileMetrics, thresholds: &Thresholds, out: &mut Vec<Finding>) {
    let Some(rust) = &file.rust else {
        return;
    };

    for function in &rust.functions {
        if function.lines() >= thresholds.long_function_lines {
            out.push(finding(
                Rule::LongFunction,
                Severity::Medium,
                format!("{} is {} lines", function.qualified_name, function.lines()),
                format!(
                    "{} spans lines {}–{} with {} parameters, against a threshold of {} lines.",
                    function.qualified_name,
                    function.start_line,
                    function.end_line,
                    function.parameters,
                    thresholds.long_function_lines
                ),
                "Lift each block that has a name you could say out loud into its own function; the ones that need three or more locals from the caller usually want to be a small struct instead.".to_owned(),
                at_line(&file.path, function.start_line),
                metric(function.lines()),
            ));
        }

        if function.complexity >= thresholds.high_complexity {
            out.push(finding(
                Rule::ComplexFunction,
                Severity::High,
                format!(
                    "{} has {} paths through it",
                    function.qualified_name, function.complexity
                ),
                format!(
                    "Cyclomatic complexity {} against a threshold of {}, nested {} blocks deep.",
                    function.complexity, thresholds.high_complexity, function.max_nesting
                ),
                "Replace the branch ladder with an early return per failure case, then push what remains into a `match` on a type that makes the impossible states unrepresentable.".to_owned(),
                at_line(&file.path, function.start_line),
                f64::from(function.complexity),
            ));
        }

        if function.max_nesting >= 5 {
            out.push(finding(
                Rule::DeepNesting,
                Severity::Medium,
                format!(
                    "{} nests {} blocks deep",
                    function.qualified_name, function.max_nesting
                ),
                format!(
                    "The deepest block in {} sits {} levels in.",
                    function.qualified_name, function.max_nesting
                ),
                "Invert the conditions and return early, or use `let ... else` for the guard cases; each level removed is a level the reader no longer has to hold.".to_owned(),
                at_line(&file.path, function.start_line),
                metric(function.max_nesting),
            ));
        }
    }

    if rust.performance.allocations_in_loops > 0 {
        out.push(finding(
            Rule::AllocationInLoop,
            Severity::High,
            format!(
                "{} allocates {} times inside loops",
                file.path, rust.performance.allocations_in_loops
            ),
            format!(
                "{} allocating calls occur inside loop bodies, so their cost is paid once per iteration.",
                rust.performance.allocations_in_loops
            ),
            "Hoist the allocation out of the loop and reuse the buffer with `clear()`, take `&str` instead of `String` at the boundary, or size the collection once with `with_capacity` before the loop.".to_owned(),
            at_file(&file.path),
            metric(rust.performance.allocations_in_loops),
        ));
    }

    if rust.performance.nested_loops > 0 {
        out.push(finding(
            Rule::NestedLoop,
            Severity::Medium,
            format!(
                "{} has {} nested loops",
                file.path, rust.performance.nested_loops
            ),
            format!(
                "{} loops sit directly inside another loop, which is quadratic in the inputs unless the inner range is bounded.",
                rust.performance.nested_loops
            ),
            "If the inner loop is a lookup, build a `HashMap` once before the outer loop and index it; that trades a quadratic scan for a linear one.".to_owned(),
            at_file(&file.path),
            metric(rust.performance.nested_loops),
        ));
    }

    if !file.is_test && rust.performance.unwraps > 0 {
        out.push(finding(
            Rule::PanicPath,
            Severity::Medium,
            format!(
                "{} has {} panic paths",
                file.path, rust.performance.unwraps
            ),
            format!(
                "{} calls to `unwrap` or `expect` outside test code, each one a way for this to abort at runtime.",
                rust.performance.unwraps
            ),
            "Return a `Result` with a specific error variant instead; keep `expect` only where the invariant is genuinely local, and give it a message that states that invariant.".to_owned(),
            at_file(&file.path),
            metric(rust.performance.unwraps),
        ));
    }

    if rust.todo_markers > 0 {
        out.push(finding(
            Rule::UnfinishedWork,
            Severity::Low,
            format!("{} has {} unfinished-work markers", file.path, rust.todo_markers),
            format!(
                "{} `TODO`, `FIXME`, `HACK`, or `XXX` markers in comments.",
                rust.todo_markers
            ),
            "Turn each one into a tracked issue and reference it from the comment, or delete it; a marker nobody is going to act on is noise that hides the ones somebody would.".to_owned(),
            at_file(&file.path),
            metric(rust.todo_markers),
        ));
    }
}

/// Flags directories holding more files than they can explain.
fn directories(
    directories: &[DirectoryMetrics],
    thresholds: &Thresholds,
    out: &mut Vec<Finding>,
) {
    for directory in directories {
        if directory.files < thresholds.large_directory_files {
            continue;
        }

        out.push(finding(
            Rule::LargeDirectory,
            Severity::Low,
            format!("{} holds {} files", directory.path, directory.files),
            format!(
                "{} files and {} lines directly inside {}, against a threshold of {} files.",
                directory.files, directory.lines.total, directory.path, thresholds.large_directory_files
            ),
            "Group the files by the thing they do into subdirectories with their own module roots; a flat directory this size means the module boundary is missing rather than large.".to_owned(),
            at_file(&directory.path),
            metric(directory.files),
        ));
    }
}

/// Flags heavy, duplicated, and apparently unused dependencies.
fn dependencies(
    report: &DependencyReport,
    thresholds: &Thresholds,
    out: &mut Vec<Finding>,
) {
    for package in report.heaviest_direct() {
        if package.exclusive_count < thresholds.heavy_dependency_crates {
            continue;
        }

        out.push(finding(
            Rule::HeavyDependency,
            Severity::Medium,
            format!(
                "{} brings {} crates of its own",
                package.name, package.exclusive_count
            ),
            format!(
                "{} v{} reaches {} crates, {} of which are in the build for no other reason.",
                package.name, package.version, package.transitive_count, package.exclusive_count
            ),
            "Check whether a narrower feature set covers what you use — `default-features = false` plus the two or three features you need routinely removes most of a subtree.".to_owned(),
            None,
            metric(package.exclusive_count),
        ));
    }

    for duplicate in &report.duplicates {
        out.push(finding(
            Rule::DuplicateDependency,
            Severity::Medium,
            format!(
                "{} is compiled at {} versions",
                duplicate.name,
                duplicate.versions.len()
            ),
            format!(
                "{} resolves to {}. Each version is compiled and linked separately, and their types do not interoperate.",
                duplicate.name,
                duplicate.versions.join(", ")
            ),
            "Find which dependency pins the older version with `cargo tree -i <crate>`, then raise its requirement or update it so both sides unify on one.".to_owned(),
            None,
            metric(duplicate.versions.len()),
        ));
    }

    for unused in &report.unused {
        out.push(finding(
            Rule::UnusedDependency,
            Severity::Low,
            format!("{} may not use {}", unused.package, unused.dependency),
            format!(
                "{} declares {} as a {} dependency, but no source file in it names the crate.",
                unused.package,
                unused.dependency,
                unused.kind.label()
            ),
            "Remove it and build. If the build still passes, it was costing compile time for nothing; if it fails, the crate is reached through a macro and belongs in `ignore_unused`.".to_owned(),
            None,
            1.0,
        ));
    }
}

/// Summarizes unreferenced items.
///
/// One finding for the whole list rather than one per item: a hundred separate
/// findings would bury every other rule, and the action — read the list, delete
/// what is genuinely dead — is the same for all of them.
fn dead_code(candidates: &[DeadCodeCandidate], out: &mut Vec<Finding>) {
    let certain = candidates
        .iter()
        .filter(|candidate| candidate.confidence == Confidence::High)
        .count();

    if certain == 0 {
        return;
    }

    let example = candidates
        .iter()
        .find(|candidate| candidate.confidence == Confidence::High);

    out.push(finding(
        Rule::DeadCode,
        Severity::High,
        format!("{certain} items are referenced by nothing"),
        format!(
            "{certain} private items have no reference anywhere in the workspace, out of {} candidates in total.",
            candidates.len()
        ),
        "Delete them. Anything worth keeping for later is in the history; anything reached only through a macro belongs in the dead-code `ignore` list so the report stays worth reading.".to_owned(),
        example.and_then(|candidate| at_line(&candidate.file, candidate.line)),
        metric(certain),
    ));
}

/// Flags files the parser refused.
///
/// A parse failure is reported rather than swallowed because it silently
/// removes a file from every Rust-level measurement in the report, and a
/// missing file reads as a clean one.
fn parse_failures(failures: &[ParseFailureReport], out: &mut Vec<Finding>) {
    for failure in failures {
        out.push(finding(
            Rule::ParseFailure,
            Severity::Low,
            format!("{} could not be parsed", failure.path),
            format!("Line {}: {}", failure.line, failure.message),
            "The file is counted in the line totals but absent from every item, complexity, and dead-code measurement. Check whether it uses syntax newer than this tool's parser.".to_owned(),
            at_line(&failure.path, failure.line),
            1.0,
        ));
    }
}

#[cfg(test)]
mod test;
