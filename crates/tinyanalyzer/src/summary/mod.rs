//! The non-interactive report.
//!
//! What `--output summary` prints: the same analysis the dashboard shows,
//! flattened into text that survives a pipe, a log, and a pull request comment.
//! It is deliberately the same ranking the dashboard opens on, so the two never
//! disagree about what the worst file in the repository is.
//!
//! Rendering is a pure function of the report and the test filter. Nothing here
//! touches a terminal, which is what makes every line of it testable by
//! comparing strings.

use std::fmt::Write as _;
use tinyanalyzer_core::{Report, Totals};

/// How many rows each ranked section prints.
const SECTION_ROWS: usize = 10;

/// Renders the whole report as text.
///
/// With `hide_tests` set, every total and every ranking counts production code
/// only — which is usually the honest answer to "how big is this project".
#[must_use]
pub fn render(report: &Report, hide_tests: bool) -> String {
    let mut out = String::new();

    header(&mut out, report);
    totals(&mut out, report, hide_tests);
    languages(&mut out, report);
    heaviest_files(&mut out, report, hide_tests);
    dependencies(&mut out, report);
    dead_code(&mut out, report);
    findings(&mut out, report);

    out
}

/// Writes a section heading.
fn section(out: &mut String, title: &str) {
    let _ = write!(out, "\n{title}\n{}\n", "─".repeat(title.chars().count()));
}

/// Renders a byte count at human scale.
///
/// Sizes in this report span single-line manifests and megabyte generated
/// sources, and a column of raw byte counts is unreadable across that range.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

    // The loss is the point: this renders a size for a human to read, and the
    // scaled value is printed to one decimal place either way.
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Shortens a path to fit a column, keeping the end.
///
/// The end of a path is the part that identifies the file; truncating from the
/// front is what a reader would do themselves.
#[must_use]
pub fn truncate_path(path: &str, width: usize) -> String {
    let length = path.chars().count();
    if length <= width || width <= 1 {
        return path.to_owned();
    }

    let keep = width.saturating_sub(1);
    let skip = length.saturating_sub(keep);

    format!("…{}", path.chars().skip(skip).collect::<String>())
}

fn header(out: &mut String, report: &Report) {
    let _ = writeln!(out, "{}", report.project.name);
    if let Some(description) = &report.project.description {
        let _ = writeln!(out, "{description}");
    }
    let _ = writeln!(out, "{}", report.project.root);
}

fn totals(out: &mut String, report: &Report, hide_tests: bool) {
    let totals: Totals = if hide_tests {
        report.production_totals()
    } else {
        report.totals
    };

    section(
        out,
        if hide_tests {
            "Totals (excluding tests)"
        } else {
            "Totals"
        },
    );

    let _ = writeln!(out, "  {:<22}{:>10}", "files", totals.files);
    let _ = writeln!(out, "  {:<22}{:>10}", "directories", totals.directories);
    let _ = writeln!(out, "  {:<22}{:>10}", "lines of code", totals.lines.code);
    let _ = writeln!(
        out,
        "  {:<22}{:>10}",
        "lines of comment", totals.lines.comment
    );
    let _ = writeln!(out, "  {:<22}{:>10}", "blank lines", totals.lines.blank);
    let _ = writeln!(out, "  {:<22}{:>10}", "functions", totals.functions);
    let _ = writeln!(out, "  {:<22}{:>10}", "items", totals.items.total());
    let _ = writeln!(out, "  {:<22}{:>10}", "on disk", human_bytes(totals.bytes));

    if !hide_tests {
        let _ = writeln!(out, "  {:<22}{:>10}", "test files", totals.test_files);
    }
    if totals.packages > 0 {
        let _ = writeln!(
            out,
            "  {:<22}{:>10}",
            "external crates", totals.external_packages
        );
    }
}

fn languages(out: &mut String, report: &Report) {
    if report.languages.is_empty() {
        return;
    }

    section(out, "Languages");

    for language in report.languages.iter().take(SECTION_ROWS) {
        let _ = writeln!(
            out,
            "  {:<14}{:>7} files{:>10} lines",
            language.language.label(),
            language.files,
            language.lines.code
        );
    }
}

fn heaviest_files(out: &mut String, report: &Report, hide_tests: bool) {
    section(out, "Heaviest files");

    let ranked: Vec<_> = report
        .files
        .iter()
        .filter(|file| !(hide_tests && file.is_test))
        .take(SECTION_ROWS)
        .collect();

    if ranked.is_empty() {
        let _ = writeln!(out, "  (none)");
        return;
    }

    for file in ranked {
        let complexity: u32 = file
            .rust
            .as_ref()
            .map(|rust| {
                rust.functions
                    .iter()
                    .map(|function| function.complexity)
                    .sum()
            })
            .unwrap_or_default();

        let _ = writeln!(
            out,
            "  {:<52}{:>6} loc{:>6} cx",
            truncate_path(&file.path, 52),
            file.lines.code,
            complexity
        );
    }
}

fn dependencies(out: &mut String, report: &Report) {
    if report.dependencies.packages.is_empty() {
        return;
    }

    section(out, "Heaviest direct dependencies");

    let heaviest = report.dependencies.heaviest_direct();
    if heaviest.is_empty() {
        let _ = writeln!(out, "  (none)");
    }

    for package in heaviest.iter().take(SECTION_ROWS) {
        let _ = writeln!(
            out,
            "  {:<34}{:<12}{:>4} exclusive{:>6} reached",
            truncate_path(&package.name, 34),
            package.version,
            package.exclusive_count,
            package.transitive_count
        );
    }

    if !report.dependencies.duplicates.is_empty() {
        section(out, "Duplicated crates");
        for duplicate in &report.dependencies.duplicates {
            let _ = writeln!(
                out,
                "  {:<34}{}",
                duplicate.name,
                duplicate.versions.join(", ")
            );
        }
    }
}

fn dead_code(out: &mut String, report: &Report) {
    if report.dead_code.is_empty() {
        return;
    }

    section(out, "Unreferenced items");

    for candidate in report.dead_code.iter().take(SECTION_ROWS) {
        let _ = writeln!(
            out,
            "  {:<8}{:<12}{:<26}{}:{}",
            candidate.confidence.label(),
            candidate.kind.label(),
            candidate.name,
            truncate_path(&candidate.file, 40),
            candidate.line
        );
    }

    if report.dead_code.len() > SECTION_ROWS {
        let _ = writeln!(
            out,
            "  … and {} more",
            report.dead_code.len() - SECTION_ROWS
        );
    }
}

fn findings(out: &mut String, report: &Report) {
    section(out, "Findings");

    if report.findings.is_empty() {
        let _ = writeln!(out, "  Nothing to report.");
        return;
    }

    for finding in report.findings.iter().take(SECTION_ROWS) {
        let _ = writeln!(out, "  [{}] {}", finding.severity.label(), finding.title);
        let _ = writeln!(out, "      {}", finding.detail);
        let _ = writeln!(out, "      → {}", finding.suggestion);
    }

    if report.findings.len() > SECTION_ROWS {
        let _ = writeln!(
            out,
            "\n  {} more findings; open the dashboard to see them all.",
            report.findings.len() - SECTION_ROWS
        );
    }
}

#[cfg(test)]
mod test;
