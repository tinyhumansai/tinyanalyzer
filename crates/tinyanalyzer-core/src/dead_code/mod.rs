//! Finding items nothing references.
//!
//! The method is a workspace-wide identifier census. Every `.rs` file
//! contributes the count of each identifier in its token stream; every
//! definition contributes one occurrence of its own name, at its own
//! declaration. An item whose name appears exactly as many times as it is
//! defined, and never more, is referenced by nothing.
//!
//! Counting tokens rather than resolving names is a deliberate trade. A real
//! resolver would need to be a compiler; this needs to run in under a second on
//! a large repository and be right often enough to be worth reading. What it
//! buys, specifically, is seeing through macros: a helper called only from
//! inside a `macro_rules!` body is invisible to an AST walk and perfectly
//! visible to a token count.
//!
//! What it costs is honesty about two things, both of which the output states
//! rather than hides:
//!
//! - **Shadowing.** Two unrelated items with the same name in different modules
//!   vouch for each other. The census cannot tell them apart, so it under-reports
//!   rather than over-reports — which is the right direction for a list a human
//!   is going to act on.
//! - **Public items.** Everything a library exports may be called from outside
//!   the workspace entirely, so those come back at [`Confidence::Medium`] and are
//!   never presented as certain.
//!
//! Modules are excluded from the report on purpose. A `mod` declaration is a
//! namespace, not a symbol: it compiles its contents whether or not anything
//! names the module itself, so "unreferenced module" would be true of almost
//! every module in a well-organized crate and would mean nothing.

mod types;

pub use types::{Confidence, DeadCodeCandidate, DeadCodeInput};

use crate::config::DeadCodeConfig;
use crate::rust_source::DefinitionKind;
use std::collections::{BTreeMap, BTreeSet};

/// Finds every item nothing in `files` appears to reference.
///
/// Returns an empty list when [`DeadCodeConfig::enabled`] is false, so a caller
/// never has to branch on the setting itself.
///
/// Results are sorted by confidence, then by file and line, so the list opens on
/// what is most likely to be genuinely removable.
#[must_use]
pub fn analyze(files: &[DeadCodeInput<'_>], config: &DeadCodeConfig) -> Vec<DeadCodeCandidate> {
    if !config.enabled {
        return Vec::new();
    }

    let ignored: BTreeSet<&str> = config.ignore.iter().map(String::as_str).collect();
    let counted: Vec<&DeadCodeInput<'_>> = files
        .iter()
        .filter(|file| config.tests_count_as_uses || !file.is_test_file)
        .collect();

    let occurrences = census(&counted);
    let declarations = declaration_counts(&counted);

    let mut candidates = Vec::new();

    // Candidates come from the same files the census counted. A file excluded
    // from the census contributed no occurrences, so every item in it would
    // trivially look unreferenced — which is an artifact of the exclusion, not
    // a finding.
    for file in &counted {
        for definition in &file.rust.definitions {
            if definition.kind == DefinitionKind::Module
                || definition.is_externally_reachable
                || ignored.contains(definition.name.as_str())
            {
                continue;
            }

            let declared = declarations
                .get(definition.name.as_str())
                .copied()
                .unwrap_or(0);
            let seen = occurrences
                .get(definition.name.as_str())
                .copied()
                .unwrap_or(0);

            if seen > declared {
                continue;
            }

            let (confidence, reason) = if definition.is_public {
                (
                    Confidence::Medium,
                    format!(
                        "no file in this workspace names `{}`, but it is public and may have callers outside it",
                        definition.name
                    ),
                )
            } else {
                (
                    Confidence::High,
                    format!(
                        "`{}` is private to this workspace and nothing here names it",
                        definition.name
                    ),
                )
            };

            candidates.push(DeadCodeCandidate {
                name: definition.name.clone(),
                kind: definition.kind,
                file: file.path.to_owned(),
                line: definition.line,
                is_public: definition.is_public,
                is_test: definition.is_test,
                confidence,
                reason,
            });
        }
    }

    candidates.sort_by(|left, right| {
        left.confidence
            .cmp(&right.confidence)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.name.cmp(&right.name))
    });

    candidates
}

/// Total identifier occurrences across every counted file.
fn census(files: &[&DeadCodeInput<'_>]) -> BTreeMap<String, usize> {
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();

    for file in files {
        for (name, count) in &file.rust.identifier_uses {
            *totals.entry(name.clone()).or_insert(0) += count;
        }
    }

    totals
}

/// How many times each name is *declared*, across the files the census counted.
///
/// Subtracting this from the census is what turns "appears in the source" into
/// "is referenced": a definition always contributes one occurrence of its own
/// name, at the point where it is introduced.
fn declaration_counts(files: &[&DeadCodeInput<'_>]) -> BTreeMap<String, usize> {
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();

    for file in files {
        for definition in &file.rust.definitions {
            *totals.entry(definition.name.clone()).or_insert(0) += 1;
        }
    }

    totals
}

#[cfg(test)]
mod test;
