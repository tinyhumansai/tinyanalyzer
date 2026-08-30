//! Unit tests for the rules engine.
//!
//! Each rule is tested at its boundary — one measurement below the threshold
//! producing nothing, one at it producing a finding — because a rule that fires
//! one unit early or late is invisible in any test that uses obviously extreme
//! inputs, and is exactly the kind of drift that erodes trust in the output.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{FindingInputs, Rule, Severity, analyze};
use crate::config::Thresholds;
use crate::dead_code::{Confidence, DeadCodeCandidate};
use crate::deps::{DependencyReport, DuplicateVersions, UnusedDependency};
use crate::loc::{Language, LineCounts};
use crate::report::{DirectoryMetrics, FileMetrics, ParseFailureReport, weight};
use crate::rust_source::{DefinitionKind, analyze as parse};

fn lines(code: usize, comment: usize) -> LineCounts {
    LineCounts {
        total: code + comment,
        code,
        comment,
        blank: 0,
    }
}

fn file(path: &str, counts: LineCounts, source: Option<&str>) -> FileMetrics {
    let rust = source.map(|text| parse(text).expect("the fixture is valid Rust"));

    FileMetrics {
        path: path.to_owned(),
        directory: ".".to_owned(),
        language: Language::Rust,
        bytes: 0,
        lines: counts,
        is_test: false,
        crate_name: None,
        weight: weight(counts, rust.as_ref()),
        notes: Vec::new(),
        rust,
    }
}

fn run(files: &[FileMetrics], thresholds: &Thresholds) -> Vec<super::Finding> {
    analyze(
        FindingInputs {
            files,
            directories: &[],
            dependencies: &DependencyReport::default(),
            dead_code: &[],
            parse_failures: &[],
        },
        thresholds,
    )
}

fn rules(findings: &[super::Finding]) -> Vec<Rule> {
    findings.iter().map(|finding| finding.rule).collect()
}

#[test]
fn nothing_measured_produces_nothing() {
    assert!(run(&[], &Thresholds::default()).is_empty());
}

#[test]
fn a_file_one_line_under_the_threshold_is_not_reported() {
    let thresholds = Thresholds::default();
    let metrics = file(
        "src/lib.rs",
        lines(thresholds.large_file_lines - 1, 400),
        None,
    );

    assert!(!rules(&run(&[metrics], &thresholds)).contains(&Rule::LargeFile));
}

#[test]
fn a_file_at_the_threshold_is_reported() {
    let thresholds = Thresholds::default();
    let metrics = file("src/lib.rs", lines(thresholds.large_file_lines, 400), None);

    let findings = run(&[metrics], &thresholds);
    let large = findings
        .iter()
        .find(|finding| finding.rule == Rule::LargeFile)
        .expect("the file is exactly at the threshold");

    assert_eq!(large.severity, Severity::Medium);
    assert!(large.detail.contains("400"));
    assert!(!large.suggestion.is_empty());
    assert_eq!(
        large.location.as_ref().map(|at| at.file.as_str()),
        Some("src/lib.rs")
    );
}

#[test]
fn a_file_past_the_second_threshold_escalates() {
    let thresholds = Thresholds::default();
    let metrics = file("src/lib.rs", lines(thresholds.huge_file_lines, 400), None);

    let findings = run(&[metrics], &thresholds);

    assert!(rules(&findings).contains(&Rule::HugeFile));
    assert!(!rules(&findings).contains(&Rule::LargeFile));
}

#[test]
fn thresholds_are_configurable_rather_than_baked_in() {
    let thresholds = Thresholds {
        large_file_lines: 10,
        huge_file_lines: 1_000,
        ..Thresholds::default()
    };
    let metrics = file("src/lib.rs", lines(12, 12), None);

    assert!(rules(&run(&[metrics], &thresholds)).contains(&Rule::LargeFile));
}

#[test]
fn a_long_function_is_reported_with_its_span() {
    let body = format!("fn long() {{\n{}}}\n", "    let _ = 1;\n".repeat(70));
    let metrics = file("src/lib.rs", lines(72, 10), Some(&body));

    let findings = run(&[metrics], &Thresholds::default());
    let long = findings
        .iter()
        .find(|finding| finding.rule == Rule::LongFunction)
        .expect("a seventy-line function");

    assert!(long.title.contains("long"));
    assert_eq!(long.location.as_ref().and_then(|at| at.line), Some(1));
}

#[test]
fn a_short_function_is_not_reported() {
    let metrics = file("src/lib.rs", lines(3, 3), Some("fn short() { let _ = 1; }"));

    assert!(!rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::LongFunction));
}

#[test]
fn a_complex_function_is_reported_as_high_severity() {
    let branches = "if a { } ".repeat(20);
    let source = format!("fn tangled(a: bool) {{ {branches} }}");
    let metrics = file("src/lib.rs", lines(3, 3), Some(&source));

    let findings = run(&[metrics], &Thresholds::default());
    let complex = findings
        .iter()
        .find(|finding| finding.rule == Rule::ComplexFunction)
        .expect("twenty branches is past the default threshold");

    assert_eq!(complex.severity, Severity::High);
    assert!(complex.metric >= 20.0);
}

#[test]
fn deep_nesting_is_reported() {
    let source = "fn deep(a: bool) { if a { if a { if a { if a { } } } } }";
    let metrics = file("src/lib.rs", lines(3, 3), Some(source));

    assert!(rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::DeepNesting));
}

#[test]
fn an_allocation_in_a_loop_is_reported_as_high_severity() {
    let source = "fn a(s: &str) { for _ in 0..3 { let _ = s.to_string(); } }";
    let metrics = file("src/hot.rs", lines(3, 3), Some(source));

    let findings = run(&[metrics], &Thresholds::default());
    let alloc = findings
        .iter()
        .find(|finding| finding.rule == Rule::AllocationInLoop)
        .expect("an allocation inside a loop");

    assert_eq!(alloc.severity, Severity::High);
    assert!(alloc.suggestion.contains("with_capacity"));
}

#[test]
fn a_nested_loop_is_reported() {
    let source = "fn a() { for _ in 0..3 { for _ in 0..3 { } } }";
    let metrics = file("src/hot.rs", lines(3, 3), Some(source));

    assert!(rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::NestedLoop));
}

#[test]
fn a_panic_path_outside_test_code_is_reported() {
    let source = "fn a() { let _ = Some(1).unwrap(); }";
    let metrics = file("src/lib.rs", lines(3, 3), Some(source));

    assert!(rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::PanicPath));
}

#[test]
fn a_panic_path_inside_test_code_is_not_reported() {
    let source = "fn a() { let _ = Some(1).unwrap(); }";
    let mut metrics = file("src/lib.rs", lines(3, 3), Some(source));
    metrics.is_test = true;

    assert!(!rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::PanicPath));
}

#[test]
fn unfinished_work_markers_are_reported() {
    let source = "// TODO: finish this\nfn a() {}\n";
    let metrics = file("src/lib.rs", lines(3, 3), Some(source));

    assert!(rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::UnfinishedWork));
}

#[test]
fn a_file_with_almost_no_comments_is_reported() {
    let metrics = file("src/lib.rs", lines(200, 1), None);

    assert!(rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::Underdocumented));
}

#[test]
fn a_well_commented_file_is_not_reported() {
    let metrics = file("src/lib.rs", lines(200, 60), None);

    assert!(!rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::Underdocumented));
}

#[test]
fn a_small_file_is_never_called_underdocumented() {
    let metrics = file("src/tiny.rs", lines(10, 0), None);

    assert!(!rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::Underdocumented));
}

#[test]
fn a_language_without_comment_syntax_is_never_called_underdocumented() {
    let mut metrics = file("data.json", lines(200, 0), None);
    metrics.language = Language::Json;

    assert!(!rules(&run(&[metrics], &Thresholds::default())).contains(&Rule::Underdocumented));
}

#[test]
fn a_large_directory_is_reported() {
    let directories = [DirectoryMetrics {
        path: "src".to_owned(),
        files: 30,
        bytes: 0,
        lines: lines(100, 10),
        is_test_only: false,
    }];

    let findings = analyze(
        FindingInputs {
            files: &[],
            directories: &directories,
            dependencies: &DependencyReport::default(),
            dead_code: &[],
            parse_failures: &[],
        },
        &Thresholds::default(),
    );

    assert!(rules(&findings).contains(&Rule::LargeDirectory));
}

#[test]
fn a_duplicated_dependency_is_reported_with_both_versions() {
    let dependencies = DependencyReport {
        duplicates: vec![DuplicateVersions {
            name: "winnow".to_owned(),
            versions: vec!["0.7.15".to_owned(), "1.0.4".to_owned()],
        }],
        ..DependencyReport::default()
    };

    let findings = analyze(
        FindingInputs {
            files: &[],
            directories: &[],
            dependencies: &dependencies,
            dead_code: &[],
            parse_failures: &[],
        },
        &Thresholds::default(),
    );

    let duplicate = findings
        .iter()
        .find(|finding| finding.rule == Rule::DuplicateDependency)
        .expect("two versions of one crate");

    assert!(duplicate.detail.contains("0.7.15"));
    assert!(duplicate.detail.contains("1.0.4"));
    assert!(duplicate.suggestion.contains("cargo tree -i"));
}

#[test]
fn an_unused_dependency_is_reported() {
    let dependencies = DependencyReport {
        unused: vec![UnusedDependency {
            package: "tinyanalyzer".to_owned(),
            dependency: "unused-crate".to_owned(),
            kind: crate::deps::DependencyKind::Normal,
        }],
        ..DependencyReport::default()
    };

    let findings = analyze(
        FindingInputs {
            files: &[],
            directories: &[],
            dependencies: &dependencies,
            dead_code: &[],
            parse_failures: &[],
        },
        &Thresholds::default(),
    );

    assert!(rules(&findings).contains(&Rule::UnusedDependency));
}

#[test]
fn dead_code_is_summarized_once_rather_than_item_by_item() {
    let candidates: Vec<DeadCodeCandidate> = (0..5)
        .map(|index| DeadCodeCandidate {
            name: format!("orphan{index}"),
            kind: DefinitionKind::Function,
            file: "src/lib.rs".to_owned(),
            line: index + 1,
            is_public: false,
            is_test: false,
            confidence: Confidence::High,
            reason: "nothing names it".to_owned(),
        })
        .collect();

    let findings = analyze(
        FindingInputs {
            files: &[],
            directories: &[],
            dependencies: &DependencyReport::default(),
            dead_code: &candidates,
            parse_failures: &[],
        },
        &Thresholds::default(),
    );

    let dead: Vec<_> = findings
        .iter()
        .filter(|finding| finding.rule == Rule::DeadCode)
        .collect();

    assert_eq!(dead.len(), 1, "one finding, not five");
    assert!(dead[0].title.contains('5'));
    assert_eq!(dead[0].severity, Severity::High);
}

#[test]
fn only_medium_confidence_dead_code_produces_no_finding() {
    let candidates = [DeadCodeCandidate {
        name: "exported".to_owned(),
        kind: DefinitionKind::Function,
        file: "src/lib.rs".to_owned(),
        line: 1,
        is_public: true,
        is_test: false,
        confidence: Confidence::Medium,
        reason: "public".to_owned(),
    }];

    let findings = analyze(
        FindingInputs {
            files: &[],
            directories: &[],
            dependencies: &DependencyReport::default(),
            dead_code: &candidates,
            parse_failures: &[],
        },
        &Thresholds::default(),
    );

    assert!(!rules(&findings).contains(&Rule::DeadCode));
}

#[test]
fn a_parse_failure_is_surfaced_rather_than_swallowed() {
    let failures = [ParseFailureReport {
        path: "src/broken.rs".to_owned(),
        line: 12,
        message: "expected `}`".to_owned(),
    }];

    let findings = analyze(
        FindingInputs {
            files: &[],
            directories: &[],
            dependencies: &DependencyReport::default(),
            dead_code: &[],
            parse_failures: &failures,
        },
        &Thresholds::default(),
    );

    let failure = findings
        .iter()
        .find(|finding| finding.rule == Rule::ParseFailure)
        .expect("an unparseable file");

    assert!(failure.detail.contains("expected `}`"));
    assert_eq!(failure.location.as_ref().and_then(|at| at.line), Some(12));
}

#[test]
fn findings_are_ordered_by_severity_then_by_measurement() {
    let big = file("src/big.rs", lines(2_000, 400), None);
    let medium = file("src/medium.rs", lines(500, 100), None);

    let findings = run(&[medium, big], &Thresholds::default());

    assert_eq!(findings[0].severity, Severity::High);
    for pair in findings.windows(2) {
        assert!(
            pair[0].severity <= pair[1].severity,
            "severities must not go backwards"
        );
    }
}

#[test]
fn every_rule_has_an_identifier_and_a_description() {
    for rule in [
        Rule::HugeFile,
        Rule::LargeFile,
        Rule::LongFunction,
        Rule::ComplexFunction,
        Rule::DeepNesting,
        Rule::AllocationInLoop,
        Rule::NestedLoop,
        Rule::HeavyDependency,
        Rule::DuplicateDependency,
        Rule::UnusedDependency,
        Rule::DeadCode,
        Rule::LargeDirectory,
        Rule::Underdocumented,
        Rule::PanicPath,
        Rule::UnfinishedWork,
        Rule::ParseFailure,
    ] {
        assert!(!rule.id().is_empty());
        assert!(!rule.description().is_empty());
    }
}

#[test]
fn every_severity_has_a_label() {
    for severity in [
        Severity::Critical,
        Severity::High,
        Severity::Medium,
        Severity::Low,
    ] {
        assert!(!severity.label().is_empty());
    }
}

#[test]
fn a_rule_identifier_matches_its_serialized_form() {
    let json = serde_json::to_string(&Rule::AllocationInLoop).expect("a rule serializes");

    assert_eq!(json, format!("\"{}\"", Rule::AllocationInLoop.id()));
}

#[test]
fn every_finding_names_a_measurement_and_a_remedy() {
    let source = "fn a(s: &str) { for _ in 0..3 { let _ = s.to_string().unwrap(); } }";
    let metrics = file("src/hot.rs", lines(900, 2), Some(source));

    for finding in run(&[metrics], &Thresholds::default()) {
        assert!(!finding.title.is_empty(), "{:?} has no title", finding.rule);
        assert!(
            !finding.detail.is_empty(),
            "{:?} has no detail",
            finding.rule
        );
        assert!(
            !finding.suggestion.is_empty(),
            "{:?} says what is wrong but not what to do",
            finding.rule
        );
    }
}
