//! Finding code written more than once, and what to replace it with.
//!
//! Every file is parsed with tree-sitter into a normalized tree — comments and
//! attributes gone, identifiers blinded, literals collapsed to their kind —
//! and three detection layers run over the result (`detect`): subtree
//! hashing for whole repeated fragments, a suffix array for repeated runs of
//! statements, and `MinHash` with tree edit distance for near-misses. Each
//! surviving group is then anti-unified into a [`Sketch`] of the shared code
//! that would replace it, and the groups are ranked by how much code that
//! would remove.
//!
//! The same trees also produce the [`SymbolRecord`] index: every item reduced
//! to its signature, shape, and the names it mentions.
//!
//! # What this approximates, and in which direction
//!
//! Detection is structural, not semantic. It **over-reports** code with the
//! same shape and a different meaning — two unrelated twelve-line `match`
//! blocks over different enums are one shape. It **under-reports** inside
//! macro invocations, whose bodies are token trees rather than syntax, and it
//! does not see two pieces of code that do the same thing written differently
//! (semantic, "type 4" clones). The sketch is a sketch: a reader decides
//! whether the copies should share code, and the signature is a starting
//! point rather than a patch.
//!
//! See `README.md` in this directory for the design.

pub(crate) mod detect;
mod sketch;
mod symbols;
pub(crate) mod syntax;
mod types;

pub use types::{
    CloneGroup, CloneInput, CloneInstance, CloneKind, Detector, FragmentKind, Parameter,
    ParameterKind, Sketch, SketchKind, SymbolRecord,
};

use crate::config::{CloneConfig, Thresholds};
use detect::{Candidate, Limits, Parsed};
use rayon::prelude::*;
use std::collections::BTreeSet;

/// Weight given to a group whose copies are all test code.
///
/// Duplicated fixtures are worth folding, but less urgently than duplicated
/// production logic: a bug in a copied fixture fails a test, while a bug in a
/// copied branch ships.
const TEST_WEIGHT: f64 = 0.5;

/// Weight given to a group with a read-only copy: only the editable copies
/// can move, so less of the group can be removed.
const READ_ONLY_WEIGHT: f64 = 0.5;

/// Finds every clone group across `inputs`, best first.
///
/// Groups whose copies are all read-only are dropped: nobody can act on them.
/// Groups mixing editable and read-only copies are kept, because they show
/// editable code re-implementing something that already exists.
#[must_use]
pub fn analyze(
    inputs: &[CloneInput<'_>],
    config: &CloneConfig,
    thresholds: &Thresholds,
) -> Vec<CloneGroup> {
    let files = parse_all(inputs);
    let limits = Limits {
        min_tokens: u32::try_from(thresholds.duplicate_min_tokens).unwrap_or(u32::MAX),
        min_lines: u32::try_from(thresholds.duplicate_min_lines).unwrap_or(u32::MAX),
        similarity: thresholds.duplicate_similarity,
        kinds: config.fragment_kinds.iter().copied().collect(),
    };

    let candidates = detect::run(&files, &limits);

    let mut groups: Vec<CloneGroup> = candidates
        .into_par_iter()
        .filter_map(|candidate| build(&files, candidate, config.include_tests))
        .collect();

    groups.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.instances[0].file.cmp(&right.instances[0].file))
            .then_with(|| {
                left.instances[0]
                    .start_line
                    .cmp(&right.instances[0].start_line)
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut groups = suppress_overlaps(groups);
    groups.truncate(config.max_groups);
    groups
}

/// Drops a group when most of its copies overlap the copies of a better
/// group already kept.
///
/// The suffix array reports every length a repeated run can be cut at: six
/// copies of fourteen lines, four of twenty-two, three of thirty — all the same
/// sequence of tests. They are not nested, so the detectors' own subsumption
/// keeps them all; this greedy pass, in score order, keeps the best cut.
fn suppress_overlaps(ranked: Vec<CloneGroup>) -> Vec<CloneGroup> {
    let mut kept: Vec<CloneGroup> = Vec::with_capacity(ranked.len());
    for group in ranked {
        let overlapping = group
            .instances
            .iter()
            .filter(|instance| {
                kept.iter()
                    .flat_map(|better| better.instances.iter())
                    .any(|other| {
                        other.file == instance.file
                            && other.start_line <= instance.end_line
                            && instance.start_line <= other.end_line
                    })
            })
            .count();
        if overlapping * 2 <= group.instances.len() {
            kept.push(group);
        }
    }
    kept
}

/// The symbol index over `inputs`, in file and line order.
#[must_use]
pub fn symbols(inputs: &[CloneInput<'_>]) -> Vec<SymbolRecord> {
    symbols::index(&parse_all(inputs))
}

/// Serializes a symbol index as JSON lines: one compact object per record.
///
/// # Errors
///
/// Returns [`crate::Error::Serialize`] if a record cannot be encoded.
pub fn symbols_to_json_lines(records: &[SymbolRecord]) -> crate::Result<String> {
    let mut out = String::new();
    for record in records {
        let line =
            serde_json::to_string(record).map_err(|source| crate::Error::Serialize { source })?;
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// Parses every input in parallel, keeping input order.
fn parse_all<'s>(inputs: &[CloneInput<'s>]) -> Vec<Parsed<'s>> {
    inputs
        .par_iter()
        .filter_map(|input| {
            syntax::parse(input.text).map(|tree| Parsed {
                input: *input,
                tree,
            })
        })
        .collect()
}

/// Turns a candidate into a reported group, or drops it.
fn build(
    files: &[Parsed<'_>],
    mut candidate: Candidate,
    include_tests: bool,
) -> Option<CloneGroup> {
    let is_test = |unit: &detect::Unit| {
        let parsed = &files[unit.file as usize];
        parsed.input.is_test_path || parsed.tree.nodes[unit.first() as usize].test
    };
    if !include_tests {
        candidate.units.retain(|unit| !is_test(unit));
    }
    if candidate.units.len() < 2
        || !candidate
            .units
            .iter()
            .any(|unit| files[unit.file as usize].input.editable)
    {
        return None;
    }

    let instances: Vec<CloneInstance> = candidate
        .units
        .iter()
        .map(|unit| {
            let parsed = &files[unit.file as usize];
            let (start, end) = unit.lines(files);
            CloneInstance {
                file: parsed.input.path.to_owned(),
                start_line: start as usize,
                end_line: end as usize,
                item: parsed.tree.qualified_name(unit.first()),
                editable: parsed.input.editable,
                is_test: is_test(unit),
            }
        })
        .collect();

    let copies = instances.len();
    let lines = instances[0].lines();
    let tokens = candidate.units[0].tokens(files) as usize;
    let in_tests = instances.iter().all(|instance| instance.is_test);
    let editable = instances.iter().all(|instance| instance.editable);

    // Counts here are copies and tokens in one repository, far below the range
    // where `f64` loses integer precision.
    #[allow(clippy::cast_precision_loss)]
    let mut score = (copies - 1) as f64 * tokens as f64 * candidate.similarity;
    if in_tests {
        score *= TEST_WEIGHT;
    }
    if !editable {
        score *= READ_ONLY_WEIGHT;
    }

    let first = &candidate.units[0];
    let id = syntax::mix(
        candidate.shape,
        syntax::mix(
            syntax::hash_str(files[first.file as usize].input.path),
            u64::from(first.first()),
        ),
    );

    Some(CloneGroup {
        id: format!("{id:016x}"),
        kind: candidate.kind,
        fragment: candidate.fragment,
        detectors: candidate
            .detectors
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        similarity: candidate.similarity,
        tokens,
        lines,
        lines_saved: (copies - 1).saturating_mul(lines).saturating_sub(copies),
        score,
        in_tests,
        editable,
        recursive: candidate.recursive,
        sketch: sketch::sketch(files, &candidate),
        instances,
    })
}

#[cfg(test)]
mod test;
