//! Turning a group of copies into a suggested piece of shared code.
//!
//! The method is anti-unification. The first copy is aligned against every
//! other copy, node by node; where they agree the shared code is fixed, and
//! where they differ it takes a parameter. Children of nodes that disagree in
//! count are aligned by the longest common subsequence of their shapes, so an
//! extra statement in one copy becomes one `Statements` parameter rather than
//! a cascade of mismatches.
//!
//! What the parameters are then decides what kind of shared code fits:
//! values only → a function; types only → a generic; item names → a macro;
//! adjacent copies differing in values only → a loop over a table. The
//! signature is a sketch for a reader, assembled from the differences and the
//! kinds of literal involved, and is not expected to compile unedited.

use crate::clones::detect::{Candidate, Parsed, Unit};
use crate::clones::syntax::{Class, Field, Tree};
use crate::clones::types::{FragmentKind, Parameter, ParameterKind, Sketch, SketchKind};
use std::collections::BTreeMap;

/// Longest value shown for a parameter in one copy.
const VALUE_WIDTH: usize = 48;

/// Copies at least this many, adjacent, and differing only in values become a
/// loop rather than a function.
const LOOP_MIN_COPIES: usize = 3;

/// One place the first copy differs from another copy.
#[derive(Debug, Clone, Copy)]
struct Hole {
    /// The node in the first copy.
    first: u32,
    /// The matching node in the other copy, or `None` when it has nothing
    /// there.
    other: Option<u32>,
    /// What the difference is.
    kind: ParameterKind,
    /// Whether `other` is present only in the other copy, inserted before
    /// `first` (or after it, when `first` ends the run).
    inserted: bool,
}

/// Builds the sketch for one group.
#[must_use]
pub(crate) fn sketch(files: &[Parsed<'_>], candidate: &Candidate) -> Sketch {
    let first = &candidate.units[0];
    let first_tree = &files[first.file as usize].tree;

    // node in the first copy -> (kind, value per copy)
    let mut holes: BTreeMap<u32, (ParameterKind, Vec<String>)> = BTreeMap::new();
    // Statements only some copies have, keyed by where they would go.
    let mut inserted: BTreeMap<u32, (ParameterKind, Vec<String>)> = BTreeMap::new();
    for (copy, unit) in candidate.units.iter().enumerate().skip(1) {
        let other_tree = &files[unit.file as usize].tree;
        let mut found = Vec::new();
        align_runs(
            first_tree,
            &first.nodes,
            other_tree,
            &unit.nodes,
            &mut found,
        );
        for hole in found {
            let target = if hole.inserted {
                &mut inserted
            } else {
                &mut holes
            };
            let entry = target.entry(hole.first).or_insert_with(|| {
                let mut values = vec![String::new(); candidate.units.len()];
                if !hole.inserted {
                    values[0] = shorten(first_tree.text(hole.first));
                }
                (hole.kind, values)
            });
            let value = hole
                .other
                .map(|node| shorten(other_tree.text(node)))
                .unwrap_or_default();
            let slot = &mut entry.1[copy];
            if slot.is_empty() {
                *slot = value;
            } else if !value.is_empty() {
                slot.push(' ');
                slot.push_str(&value);
            }
        }
    }

    // A copy that is a whole item is replaced as a whole, so its own name
    // differing is expected rather than a parameter.
    let own_names: Vec<u32> = first
        .nodes
        .iter()
        .filter_map(|&node| first_tree.field(node, Field::Name))
        .collect();
    holes.retain(|node, _| !own_names.contains(node));

    // Keep only the outermost hole on any path.
    let positions: Vec<u32> = holes.keys().copied().collect();
    holes.retain(|&node, _| {
        !positions
            .iter()
            .any(|&outer| outer != node && first_tree.contains(outer, node))
    });
    inserted.retain(|&node, _| {
        !positions
            .iter()
            .any(|&outer| outer != node && first_tree.contains(outer, node))
    });
    let mut differences: Vec<(u32, ParameterKind, Vec<String>)> = holes
        .iter()
        .chain(&inserted)
        .map(|(&node, (kind, values))| (node, *kind, values.clone()))
        .collect();
    differences.sort_by_key(|(node, _, _)| *node);

    let parameters = parameters(first_tree, &differences);
    let kind = choose(files, candidate, first_tree, &differences);
    let signature = signature(kind, first_tree, first, candidate, &parameters);
    let summary = summary(kind, candidate, &parameters);

    Sketch {
        kind,
        signature,
        summary,
        parameters,
    }
}

/// Aligns two sibling runs.
///
/// Runs of equal length are aligned position by position. Otherwise the
/// longest common subsequence pairs what it can; inside each gap between
/// pairs, leftover siblings are paired in order, an unpaired sibling of the
/// first copy is missing from the other, and an unpaired sibling of the other
/// copy is an insertion.
fn align_runs(a: &Tree<'_>, left: &[u32], b: &Tree<'_>, right: &[u32], out: &mut Vec<Hole>) {
    if left.len() == right.len() {
        for (&x, &y) in left.iter().zip(right) {
            align(a, x, b, y, out);
        }
        return;
    }

    let mut matched = lcs(a, left, b, right);
    matched.push((left.len(), right.len()));
    let (mut i, mut j) = (0, 0);
    for (next_i, next_j) in matched {
        let gap_left = &left[i..next_i];
        let gap_right = &right[j..next_j];
        for (offset, &x) in gap_left.iter().enumerate() {
            if !a.nodes[x as usize].named {
                continue;
            }
            out.push(Hole {
                first: x,
                other: gap_right.get(offset).copied(),
                kind: ParameterKind::Statements,
                inserted: false,
            });
        }
        let anchor = left.get(next_i).or_else(|| left.last()).copied();
        for &y in gap_right.iter().skip(gap_left.len()) {
            if let (Some(anchor), true) = (anchor, b.nodes[y as usize].named) {
                out.push(Hole {
                    first: anchor,
                    other: Some(y),
                    kind: ParameterKind::Statements,
                    inserted: true,
                });
            }
        }
        if next_i < left.len() && next_j < right.len() {
            align(a, left[next_i], b, right[next_j], out);
        }
        (i, j) = (next_i + 1, next_j + 1);
    }
}

/// Aligns two nodes, recording every difference below them.
fn align(a: &Tree<'_>, x: u32, b: &Tree<'_>, y: u32, out: &mut Vec<Hole>) {
    let (left, right) = (&a.nodes[x as usize], &b.nodes[y as usize]);
    if left.shape == right.shape && a.exact_hash(&[x]) == b.exact_hash(&[y]) {
        return;
    }
    if left.kind != right.kind {
        out.push(Hole {
            first: x,
            other: Some(y),
            kind: kind_of(a, x),
            inserted: false,
        });
        return;
    }
    if left.class != Class::Inner {
        if a.text(x) != b.text(y) {
            out.push(Hole {
                first: x,
                other: Some(y),
                kind: kind_of(a, x),
                inserted: false,
            });
        }
        return;
    }

    let left_children: Vec<u32> = a.children(x).collect();
    let right_children: Vec<u32> = b.children(y).collect();
    align_runs(a, &left_children, b, &right_children, out);
}

/// Longest common subsequence of two sibling runs, as index pairs.
///
/// Weighted: an exact pair counts twice and a same-shape pair once, so an
/// identical statement is preferred over a merely similar one when both fit.
fn lcs(
    left_tree: &Tree<'_>,
    left: &[u32],
    right_tree: &Tree<'_>,
    right: &[u32],
) -> Vec<(usize, usize)> {
    let weight = |row: usize, column: usize| {
        let (x, y) = (left[row], right[column]);
        if left_tree.nodes[x as usize].shape != right_tree.nodes[y as usize].shape {
            0
        } else if left_tree.exact_hash(&[x]) == right_tree.exact_hash(&[y]) {
            2
        } else {
            1
        }
    };
    let width = right.len() + 1;
    let mut table = vec![0_u32; (left.len() + 1) * width];
    for row in (0..left.len()).rev() {
        for column in (0..right.len()).rev() {
            let skip = table[(row + 1) * width + column].max(table[row * width + column + 1]);
            let take = match weight(row, column) {
                0 => 0,
                gain => table[(row + 1) * width + column + 1] + gain,
            };
            table[row * width + column] = skip.max(take);
        }
    }

    let mut pairs = Vec::new();
    let (mut row, mut column) = (0, 0);
    while row < left.len() && column < right.len() {
        let gain = weight(row, column);
        if gain > 0 && table[row * width + column] == table[(row + 1) * width + column + 1] + gain {
            pairs.push((row, column));
            row += 1;
            column += 1;
        } else if table[(row + 1) * width + column] >= table[row * width + column + 1] {
            row += 1;
        } else {
            column += 1;
        }
    }
    pairs
}

/// What a differing node in the first copy stands for.
fn kind_of(tree: &Tree<'_>, node: u32) -> ParameterKind {
    let data = &tree.nodes[node as usize];
    match data.class {
        Class::Literal => ParameterKind::Literal,
        Class::TypeIdent => ParameterKind::Type,
        Class::Ident => ParameterKind::Identifier,
        _ if data.kind.ends_with("_type") || data.kind == "type_identifier" => ParameterKind::Type,
        _ if data.parent != crate::clones::syntax::NONE
            && tree.nodes[data.parent as usize].kind == "block" =>
        {
            ParameterKind::Statements
        }
        _ => ParameterKind::Expression,
    }
}

/// Turns holes into parameters, folding a consistently renamed identifier into
/// one parameter however many times it appears.
fn parameters(
    tree: &Tree<'_>,
    differences: &[(u32, ParameterKind, Vec<String>)],
) -> Vec<Parameter> {
    let mut seen: BTreeMap<(ParameterKind, Vec<String>), usize> = BTreeMap::new();
    let mut parameters: Vec<Parameter> = Vec::new();
    let mut types = 0;

    for (node, kind, values) in differences {
        let key = (*kind, values.clone());
        if matches!(kind, ParameterKind::Identifier | ParameterKind::Type)
            && seen.contains_key(&key)
        {
            continue;
        }
        let name = if *kind == ParameterKind::Type {
            types += 1;
            format!("T{}", types - 1)
        } else {
            format!("p{}", parameters.len() - types)
        };
        seen.insert(key, parameters.len());
        parameters.push(Parameter {
            name,
            kind: *kind,
            line: tree.nodes[*node as usize].start_line as usize,
            values: values.clone(),
        });
    }

    parameters
}

/// Picks the kind of shared code that fits the differences.
fn choose(
    files: &[Parsed<'_>],
    candidate: &Candidate,
    tree: &Tree<'_>,
    differences: &[(u32, ParameterKind, Vec<String>)],
) -> SketchKind {
    if candidate.recursive {
        return SketchKind::Recursive;
    }
    if candidate.fragment == FragmentKind::TypeShape {
        return SketchKind::SharedType;
    }

    let values_only = differences
        .iter()
        .all(|(_, kind, _)| matches!(kind, ParameterKind::Literal | ParameterKind::Identifier));
    if values_only && !differences.is_empty() && adjacent(files, &candidate.units) {
        return SketchKind::Loop;
    }

    if differences
        .iter()
        .any(|(node, kind, _)| *kind != ParameterKind::Statements && needs_macro(tree, *node))
    {
        return SketchKind::Macro;
    }

    if !differences.is_empty()
        && differences
            .iter()
            .all(|(_, kind, _)| *kind == ParameterKind::Type)
    {
        return SketchKind::Generic;
    }

    SketchKind::Function
}

/// Whether a difference is something no function parameter can stand for: a
/// field name, or the declared name of an item, field, or variant.
fn needs_macro(tree: &Tree<'_>, node: u32) -> bool {
    let data = &tree.nodes[node as usize];
    if matches!(data.kind, "field_identifier" | "shorthand_field_identifier") {
        return true;
    }
    if data.field != Field::Name || data.parent == crate::clones::syntax::NONE {
        return false;
    }
    let parent = tree.nodes[data.parent as usize].kind;
    parent.ends_with("_item")
        || matches!(
            parent,
            "field_declaration" | "enum_variant" | "macro_definition"
        )
}

/// Whether the copies are at least [`LOOP_MIN_COPIES`] consecutive siblings in
/// one parent.
fn adjacent(files: &[Parsed<'_>], units: &[Unit]) -> bool {
    if units.len() < LOOP_MIN_COPIES {
        return false;
    }
    let file = units[0].file;
    let tree = &files[file as usize].tree;
    let parent = tree.nodes[units[0].first() as usize].parent;

    units.windows(2).all(|pair| {
        let (before, after) = (&pair[0], &pair[1]);
        if after.file != file || tree.nodes[after.first() as usize].parent != parent {
            return false;
        }
        // Between the two copies there may only be punctuation.
        let mut next = tree.nodes[before.last() as usize].end;
        while next < after.first() {
            if tree.nodes[next as usize].named {
                return false;
            }
            next = tree.nodes[next as usize].end;
        }
        next == after.first()
    })
}

/// The suggested signature or header.
fn signature(
    kind: SketchKind,
    tree: &Tree<'_>,
    first: &Unit,
    candidate: &Candidate,
    parameters: &[Parameter],
) -> String {
    let base = base_name(tree, first);
    let generics: Vec<&str> = parameters
        .iter()
        .filter(|parameter| parameter.kind == ParameterKind::Type)
        .map(|parameter| parameter.name.as_str())
        .collect();
    let values: Vec<&Parameter> = parameters
        .iter()
        .filter(|parameter| parameter.kind != ParameterKind::Type)
        .collect();
    let generic_list = if generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", generics.join(", "))
    };
    let arguments = values
        .iter()
        .map(|parameter| format!("{}: {}", parameter.name, type_hint(parameter)))
        .collect::<Vec<_>>()
        .join(", ");
    let returns = return_type(tree, first)
        .map(|ty| format!(" -> {ty}"))
        .unwrap_or_default();

    match kind {
        SketchKind::SharedType => {
            let keyword = if tree.nodes[first.first() as usize].kind == "enum_item" {
                "enum"
            } else {
                "struct"
            };
            format!("{keyword} {}{generic_list} {{ … }}", camel(&base))
        }
        SketchKind::Macro => {
            let fragments = parameters
                .iter()
                .map(|parameter| {
                    let fragment = match parameter.kind {
                        ParameterKind::Identifier => "ident",
                        ParameterKind::Literal => "literal",
                        ParameterKind::Type => "ty",
                        ParameterKind::Expression => "expr",
                        ParameterKind::Statements => "tt",
                    };
                    format!("${}:{fragment}", parameter.name)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("macro_rules! {base} {{ ({fragments}) => {{ … }} }}")
        }
        SketchKind::Loop => {
            let rows = (0..candidate.units.len())
                .map(|copy| {
                    let row: Vec<&str> = values
                        .iter()
                        .map(|parameter| parameter.values[copy].as_str())
                        .collect();
                    if row.len() == 1 {
                        row[0].to_owned()
                    } else {
                        format!("({})", row.join(", "))
                    }
                })
                .take(4)
                .collect::<Vec<_>>();
            let more = if candidate.units.len() > 4 {
                ", …"
            } else {
                ""
            };
            let binding = if values.len() == 1 {
                values[0].name.clone()
            } else {
                format!(
                    "({})",
                    values
                        .iter()
                        .map(|parameter| parameter.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            format!("for {binding} in [{}{more}] {{ … }}", rows.join(", "))
        }
        SketchKind::Function | SketchKind::Generic | SketchKind::Recursive => {
            format!("fn {base}{generic_list}({arguments}){returns}")
        }
    }
}

/// A name for the shared code, from the first copy.
///
/// A whole item lends its own name (`shared_parse`); part of a function
/// borrows the function's (`parse_step`); an impl block borrows its type's.
fn base_name(tree: &Tree<'_>, first: &Unit) -> String {
    let node = first.first();
    let whole = (first.nodes.len() == 1)
        .then(|| match tree.nodes[node as usize].kind {
            "impl_item" => tree.field(node, Field::Type).map(|ty| tree.text(ty)),
            _ => tree.name(node),
        })
        .flatten();
    let name = match whole {
        Some(name) => format!("shared_{}", identifier(name)),
        None => tree
            .enclosing(node, &["function_item"])
            .and_then(|function| tree.name(function))
            .map_or_else(
                || "shared_helper".to_owned(),
                |name| format!("{}_step", identifier(name)),
            ),
    };
    name.to_lowercase()
}

/// Reduces a name or type to an identifier: `Graph<State>` → `graph`.
fn identifier(text: &str) -> String {
    let head = text
        .trim_start_matches("r#")
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|part| !part.is_empty())
        .unwrap_or("helper");
    let mut snake = String::new();
    for (index, c) in head.chars().enumerate() {
        if c.is_uppercase() && index > 0 {
            snake.push('_');
        }
        snake.extend(c.to_lowercase());
    }
    snake
}

/// `shared_parse_config` → `SharedParseConfig`.
fn camel(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |head| {
                head.to_uppercase().chain(chars).collect()
            })
        })
        .collect()
}

/// The declared return type of a copy that is a function.
fn return_type<'s>(tree: &Tree<'s>, first: &Unit) -> Option<&'s str> {
    if first.nodes.len() != 1 {
        return None;
    }
    let node = first.first();
    (tree.nodes[node as usize].kind == "function_item")
        .then(|| tree.field(node, Field::ReturnType))
        .flatten()
        .map(|ty| tree.text(ty))
}

/// A type for a value parameter, guessed from its values.
fn type_hint(parameter: &Parameter) -> &'static str {
    match parameter.kind {
        ParameterKind::Statements => "impl FnOnce()",
        ParameterKind::Literal => {
            let sample = parameter.values.first().map_or("", String::as_str);
            if sample.starts_with('"') || sample.starts_with("r#") || sample.starts_with("r\"") {
                "&str"
            } else if sample.starts_with('\'') {
                "char"
            } else if sample == "true" || sample == "false" {
                "bool"
            } else if sample.contains('.') {
                "f64"
            } else {
                "i64"
            }
        }
        ParameterKind::Identifier | ParameterKind::Expression | ParameterKind::Type => "_",
    }
}

/// One sentence on what to do.
fn summary(kind: SketchKind, candidate: &Candidate, parameters: &[Parameter]) -> String {
    let copies = candidate.units.len();
    let differences = match parameters.len() {
        0 => "they are identical".to_owned(),
        1 => "they differ in one place".to_owned(),
        count => format!("they differ in {count} places"),
    };
    match kind {
        SketchKind::Function => format!(
            "Extract one function and call it from all {copies} places; {differences}, each of which becomes a parameter."
        ),
        SketchKind::Generic => format!(
            "Make one generic over the types that differ and use it from all {copies} places; {differences}."
        ),
        SketchKind::Macro => format!(
            "The copies differ in item or field names, which a function cannot take; one `macro_rules!` covers all {copies} — {differences}."
        ),
        SketchKind::Loop => format!(
            "These {copies} adjacent copies differ only in values; put the values in a table and loop over it."
        ),
        SketchKind::Recursive => format!(
            "One copy sits inside another; a helper that calls itself covers all {copies} — {differences}."
        ),
        SketchKind::SharedType => format!(
            "{copies} definitions have the same field types in the same order; declare it once (generic if the types differ) and reuse it — {differences}."
        ),
    }
}

/// Collapses whitespace and cuts a value to [`VALUE_WIDTH`] characters.
fn shorten(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= VALUE_WIDTH {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(VALUE_WIDTH - 1).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod test;
