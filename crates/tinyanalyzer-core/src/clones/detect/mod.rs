//! The detection layers, and the merge that turns their matches into groups.
//!
//! Three layers run over the same flattened trees, each catching what the
//! others miss:
//!
//! 1. **Subtree hashes** group whole fragments — functions, impls, closures,
//!    match arms, if chains, loop bodies, type definitions — whose
//!    identifier-blind shapes are equal. Exact and renamed copies, plus copies
//!    whose independent statements were reordered.
//! 2. **The suffix array** ([`suffix`]) finds repeated token runs regardless of
//!    syntax, then trims each run to the complete statements or items inside
//!    it. Copies that are part of a function rather than all of one.
//! 3. **Near-miss similarity** ([`similar`]) proposes pairs by `MinHash` and
//!    confirms them by size, node-kind Dice, and tree edit distance. Copies
//!    that gained or lost a statement.
//!
//! The merge then deduplicates groups found by more than one layer, drops a
//! copy nested in another copy of the same group (marking the group
//! recursive), and drops any group whose copies all sit inside the copies of a
//! larger group — so the report names the largest repeated thing, not every
//! piece of it.

pub(crate) mod similar;
pub(crate) mod suffix;

use crate::clones::syntax::{Class, Field, NONE, Tree, hash_str, mix};
use crate::clones::types::{CloneInput, CloneKind, Detector, FragmentKind};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

/// Fragments larger than this many nodes skip tree edit distance and are
/// confirmed by Dice and `MinHash` alone.
///
/// Zhang–Shasha is quadratic in the fragment size and worse in its shape. Past
/// a few hundred nodes it costs more than every other layer combined, and the
/// cheaper measures are reliable at that size anyway: two large fragments that
/// agree on their shingles and their node mix are near-copies.
const EDIT_DISTANCE_LIMIT: u32 = 300;

/// Occurrences kept per repeated run. A line repeated a thousand times is one
/// finding, not a thousand.
const MAX_OCCURRENCES: usize = 64;

/// LSH buckets larger than this are chained rather than fully paired.
const BUCKET_PAIR_LIMIT: usize = 32;

/// One file, parsed.
#[derive(Debug)]
pub(crate) struct Parsed<'s> {
    /// What the caller handed in.
    pub input: CloneInput<'s>,
    /// The flattened tree.
    pub tree: Tree<'s>,
}

/// One copy: a single node, or a run of adjacent siblings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Unit {
    /// Index into the parsed files.
    pub file: u32,
    /// The node, or the sibling run, in order.
    pub nodes: Vec<u32>,
}

impl Unit {
    /// The first node.
    pub(crate) fn first(&self) -> u32 {
        self.nodes.first().copied().unwrap_or(0)
    }

    /// The last node.
    pub(crate) fn last(&self) -> u32 {
        self.nodes.last().copied().unwrap_or(0)
    }

    /// Byte span, start inclusive and end exclusive.
    pub(crate) fn span(&self, files: &[Parsed<'_>]) -> (u32, u32) {
        let tree = &files[self.file as usize].tree;
        (
            tree.nodes[self.first() as usize].start_byte,
            tree.nodes[self.last() as usize].end_byte,
        )
    }

    /// One-based inclusive line span.
    pub(crate) fn lines(&self, files: &[Parsed<'_>]) -> (u32, u32) {
        let tree = &files[self.file as usize].tree;
        (
            tree.nodes[self.first() as usize].start_line,
            tree.nodes[self.last() as usize].end_line,
        )
    }

    /// Normalized tokens.
    pub(crate) fn tokens(&self, files: &[Parsed<'_>]) -> u32 {
        let tree = &files[self.file as usize].tree;
        self.nodes
            .iter()
            .map(|&node| tree.nodes[node as usize].leaf_count)
            .sum()
    }

    /// Whether `other` lies within this unit.
    fn encloses(&self, other: &Self, files: &[Parsed<'_>]) -> bool {
        let (start, end) = self.span(files);
        let (inner_start, inner_end) = other.span(files);
        self.file == other.file && start <= inner_start && inner_end <= end
    }

    /// Whether the two units share any byte.
    fn overlaps(&self, other: &Self, files: &[Parsed<'_>]) -> bool {
        let (start, end) = self.span(files);
        let (other_start, other_end) = other.span(files);
        self.file == other.file && start < other_end && other_start < end
    }
}

/// A group of copies, before ranking and sketching.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    /// The copies, sorted by file then position.
    pub units: Vec<Unit>,
    /// The granularity.
    pub fragment: FragmentKind,
    /// How alike the copies are.
    pub kind: CloneKind,
    /// Which layers found it.
    pub detectors: BTreeSet<Detector>,
    /// The lowest pairwise similarity.
    pub similarity: f64,
    /// The structural hash the group is keyed on.
    pub shape: u64,
    /// Whether a copy sat inside another copy of the same group.
    pub recursive: bool,
}

/// What the layers compare against.
#[derive(Debug, Clone)]
pub(crate) struct Limits {
    /// Fewest normalized tokens a copy may have.
    pub min_tokens: u32,
    /// Fewest lines a copy may span.
    pub min_lines: u32,
    /// Lowest similarity two near-miss copies may have.
    pub similarity: f64,
    /// Which granularities to look at.
    pub kinds: BTreeSet<FragmentKind>,
}

/// One whole-node fragment.
#[derive(Debug, Clone, Copy)]
struct Fragment {
    file: u32,
    node: u32,
    kind: FragmentKind,
    shape: u64,
}

/// Runs every enabled layer and merges the results.
#[must_use]
pub(crate) fn run(files: &[Parsed<'_>], limits: &Limits) -> Vec<Candidate> {
    let fragments: Vec<Fragment> = files
        .par_iter()
        .enumerate()
        .flat_map_iter(|(file, parsed)| fragments(to_u32(file), &parsed.tree, limits))
        .collect();

    let mut found = subtree_groups(files, &fragments);
    if limits.kinds.contains(&FragmentKind::Statements) {
        found.extend(suffix_groups(files, limits));
    }
    found.extend(near_misses(files, &fragments, limits));

    merge(files, found)
}

/// The granularity a node is a fragment at, if any.
pub(crate) fn fragment_kind(tree: &Tree<'_>, node: u32) -> Option<FragmentKind> {
    let data = &tree.nodes[node as usize];
    let parent_kind = (data.parent != NONE).then(|| tree.nodes[data.parent as usize].kind);
    match data.kind {
        "function_item" => Some(FragmentKind::Function),
        "impl_item" => Some(FragmentKind::Impl),
        "trait_item" => Some(FragmentKind::Trait),
        "closure_expression" => Some(FragmentKind::Closure),
        "match_arm" => Some(FragmentKind::MatchArm),
        "if_expression" if parent_kind != Some("else_clause") => Some(FragmentKind::IfChain),
        "block"
            if matches!(
                parent_kind,
                Some(
                    "for_expression"
                        | "while_expression"
                        | "loop_expression"
                        | "else_clause"
                        | "if_expression"
                )
            ) =>
        {
            Some(FragmentKind::Block)
        }
        "struct_item" | "enum_item" => Some(FragmentKind::TypeShape),
        _ => None,
    }
}

/// Every fragment in one file big enough to report.
fn fragments(file: u32, tree: &Tree<'_>, limits: &Limits) -> Vec<Fragment> {
    (0..to_u32(tree.nodes.len()))
        .filter_map(|node| {
            let kind = fragment_kind(tree, node)?;
            let data = &tree.nodes[node as usize];
            if data.error || !limits.kinds.contains(&kind) {
                return None;
            }
            let lines = data.end_line - data.start_line + 1;
            let big_enough = lines >= limits.min_lines
                && if kind == FragmentKind::TypeShape {
                    typed_leaves(tree, node) >= MIN_TYPED_LEAVES
                } else {
                    data.leaf_count >= limits.min_tokens
                };
            big_enough.then(|| Fragment {
                file,
                node,
                kind,
                shape: if kind == FragmentKind::TypeShape {
                    type_shape(tree, node)
                } else {
                    data.shape
                },
            })
        })
        .collect()
}

/// Type names a struct or enum must mention to be compared by shape.
///
/// A type shape blinds every name, so an enum whose variants carry no data —
/// `enum Mode { Fast, Slow, Off }` — has the same shape as every other
/// three-variant enum, whatever it means. Two type mentions is the least that
/// gives a shape something to agree on.
const MIN_TYPED_LEAVES: usize = 2;

/// Type names and primitive types mentioned in a definition, its own name
/// excluded.
fn typed_leaves(tree: &Tree<'_>, node: u32) -> usize {
    let own_name = tree.field(node, Field::Name);
    tree.leaves_of(node)
        .iter()
        .filter(|&&leaf| Some(leaf) != own_name)
        .filter(|&&leaf| {
            let data = &tree.nodes[leaf as usize];
            data.class == Class::TypeIdent || data.kind == "primitive_type"
        })
        .count()
}

/// A type definition's shape: field types kept, every name blinded.
///
/// Two structs with the same field types in the same order are one shape
/// whatever they and their fields are called — which is exactly the case for
/// a shared type.
fn type_shape(tree: &Tree<'_>, node: u32) -> u64 {
    let own_name = tree.field(node, Field::Name);
    tree.leaves_of(node)
        .iter()
        .filter(|&&leaf| Some(leaf) != own_name)
        .fold(hash_str(tree.nodes[node as usize].kind), |hash, &leaf| {
            let data = &tree.nodes[leaf as usize];
            let token = match data.class {
                Class::Ident => 1,
                Class::Literal => hash_str(data.kind),
                _ => hash_str(tree.text(leaf)),
            };
            mix(hash, token)
        })
}

/// Layer 1: whole fragments with equal shapes.
fn subtree_groups(files: &[Parsed<'_>], fragments: &[Fragment]) -> Vec<Candidate> {
    let mut by_shape: BTreeMap<(FragmentKind, u64), Vec<Unit>> = BTreeMap::new();
    for fragment in fragments {
        by_shape
            .entry((fragment.kind, fragment.shape))
            .or_default()
            .push(Unit {
                file: fragment.file,
                nodes: vec![fragment.node],
            });
    }

    by_shape
        .into_iter()
        .filter(|(_, units)| units.len() > 1)
        .map(|((fragment, shape), units)| {
            let (kind, reordered) = classify(files, &units, fragment == FragmentKind::TypeShape);
            let mut detectors = BTreeSet::from([Detector::SubtreeHash]);
            if reordered {
                detectors.insert(Detector::Reordered);
            }
            Candidate {
                units,
                fragment,
                kind,
                detectors,
                similarity: 1.0,
                shape,
                recursive: false,
            }
        })
        .collect()
}

/// How alike a set of same-shape units is, and whether reordering was needed.
fn classify(files: &[Parsed<'_>], units: &[Unit], type_shape: bool) -> (CloneKind, bool) {
    let sequence = |unit: &Unit| {
        let tree = &files[unit.file as usize].tree;
        unit.nodes
            .iter()
            .flat_map(|&node| tree.leaves_of(node).iter())
            .fold(0_u64, |hash, &leaf| mix(hash, u64::from(tree.token(leaf))))
    };
    let exact = |unit: &Unit| files[unit.file as usize].tree.exact_hash(&unit.nodes);

    let first = &units[0];
    if !type_shape && units.iter().any(|unit| sequence(unit) != sequence(first)) {
        return (CloneKind::NearMiss, true);
    }
    if units.iter().all(|unit| exact(unit) == exact(first)) {
        (CloneKind::Exact, false)
    } else {
        (CloneKind::Renamed, false)
    }
}

/// Layer 2: repeated token runs, trimmed to whole syntax.
fn suffix_groups(files: &[Parsed<'_>], limits: &Limits) -> Vec<Candidate> {
    let mut text: Vec<u32> = Vec::new();
    let mut offsets: Vec<usize> = Vec::with_capacity(files.len());
    for (index, parsed) in files.iter().enumerate() {
        offsets.push(text.len());
        text.extend(
            parsed
                .tree
                .leaves
                .iter()
                .map(|&leaf| parsed.tree.token(leaf)),
        );
        text.push((1 << 31) + to_u32(index));
    }

    let sa = suffix::suffix_array(&text);
    let lcp = suffix::lcp_array(&text, &sa);
    let repeats = suffix::repeats(&text, &sa, &lcp, limits.min_tokens, MAX_OCCURRENCES);

    let trimmed: Vec<Vec<Unit>> = repeats
        .par_iter()
        .map(|repeat| {
            repeat
                .starts
                .iter()
                .filter_map(|&start| {
                    let file = offsets.partition_point(|&offset| offset <= start as usize) - 1;
                    let local = start as usize - offsets[file];
                    let tree = &files[file].tree;
                    let nodes = trim(tree, local, local + repeat.length as usize - 1)?;
                    Some(Unit {
                        file: to_u32(file),
                        nodes,
                    })
                })
                .collect()
        })
        .collect();

    let mut by_shape: BTreeMap<u64, BTreeSet<Unit>> = BTreeMap::new();
    for unit in trimmed.into_iter().flatten() {
        let tree = &files[unit.file as usize].tree;
        let (start, end) = unit.lines(files);
        if unit.tokens(files) < limits.min_tokens
            || end - start + 1 < limits.min_lines
            || is_declarations_only(tree, &unit.nodes)
        {
            continue;
        }
        let shape = run_shape(tree, &unit.nodes);
        by_shape.entry(shape).or_default().insert(unit);
    }

    by_shape
        .into_iter()
        .filter(|(_, units)| units.len() > 1)
        .map(|(shape, units)| {
            let units: Vec<Unit> = units.into_iter().collect();
            let fragment = if units[0].nodes.len() == 1 {
                fragment_kind(&files[units[0].file as usize].tree, units[0].first())
                    .unwrap_or(FragmentKind::Statements)
            } else {
                FragmentKind::Statements
            };
            let (kind, reordered) = classify(files, &units, false);
            let mut detectors = BTreeSet::from([Detector::SuffixArray]);
            if reordered {
                detectors.insert(Detector::Reordered);
            }
            Candidate {
                units,
                fragment,
                kind,
                detectors,
                similarity: 1.0,
                shape,
                recursive: false,
            }
        })
        .collect()
}

/// The shape of a sibling run; a single node keeps its own shape, so the same
/// node found by two layers keys the same way.
fn run_shape(tree: &Tree<'_>, nodes: &[u32]) -> u64 {
    match nodes {
        [single] => tree.nodes[*single as usize].shape,
        _ => nodes.iter().fold(hash_str("run"), |hash, &node| {
            mix(hash, tree.nodes[node as usize].shape)
        }),
    }
}

/// Item kinds that only name something declared elsewhere.
const DECLARATIONS: &[&str] = &["mod_item", "use_declaration", "extern_crate_declaration"];

/// Whether a run is nothing but `mod`, `use`, and `extern crate` lines, or a
/// file or module body made only of them.
///
/// Every list of `pub mod a; pub mod b; …` has the same shape as every other,
/// and there is nothing to fold: each line names a different module.
fn is_declarations_only(tree: &Tree<'_>, nodes: &[u32]) -> bool {
    nodes
        .iter()
        .all(|&node| match tree.nodes[node as usize].kind {
            "source_file" | "declaration_list" => {
                let children: Vec<u32> = tree.named_children(node).collect();
                is_declarations_only(tree, &children)
            }
            kind => {
                DECLARATIONS.contains(&kind)
                    && tree
                        .children(node)
                        .all(|child| tree.nodes[child as usize].kind != "declaration_list")
            }
        })
}

/// Node kinds whose children form a sequence a copy can be a run of.
const SEQUENCES: &[&str] = &[
    "block",
    "source_file",
    "declaration_list",
    "match_block",
    "field_declaration_list",
    "enum_variant_list",
    "field_initializer_list",
    "arguments",
    "array_expression",
    "token_tree",
];

/// Narrows the leaves `first..=last` to the complete syntax inside them.
///
/// The answer is either one node the range covers exactly, or a run of
/// siblings in a sequence-like parent — a block's statements, an impl's
/// methods, a call's arguments. Anything else is a fragment of an expression
/// and is dropped.
fn trim(tree: &Tree<'_>, first: usize, last: usize) -> Option<Vec<u32>> {
    let start_leaf = *tree.leaves.get(first)?;
    let end_leaf = *tree.leaves.get(last)?;
    let start = tree.nodes[start_leaf as usize].start_byte;
    let end = tree.nodes[end_leaf as usize].end_byte;

    let mut ancestor = start_leaf;
    while !tree.contains(ancestor, end_leaf) {
        ancestor = tree.nodes[ancestor as usize].parent;
        if ancestor == NONE {
            return None;
        }
    }

    let data = &tree.nodes[ancestor as usize];
    if data.error {
        return None;
    }
    if data.start_byte >= start && data.end_byte <= end && data.named {
        return Some(vec![ancestor]);
    }
    if !SEQUENCES.contains(&data.kind) {
        return None;
    }

    let run: Vec<u32> = tree
        .children(ancestor)
        .filter(|&child| {
            let child = &tree.nodes[child as usize];
            child.start_byte >= start && child.end_byte <= end
        })
        .collect();
    let first_named = run
        .iter()
        .position(|&node| tree.nodes[node as usize].named)?;
    let last_named = run
        .iter()
        .rposition(|&node| tree.nodes[node as usize].named)?;
    let run = run[first_named..=last_named].to_vec();
    (!run.is_empty()).then_some(run)
}

/// Layer 3: near-miss pairs, joined into groups.
fn near_misses(files: &[Parsed<'_>], fragments: &[Fragment], limits: &Limits) -> Vec<Candidate> {
    let eligible: Vec<&Fragment> = fragments
        .iter()
        .filter(|fragment| fragment.kind != FragmentKind::TypeShape)
        .collect();

    let signatures: Vec<[u64; similar::SIGNATURE]> = eligible
        .par_iter()
        .map(|fragment| {
            let tree = &files[fragment.file as usize].tree;
            let tokens: Vec<u32> = tree
                .leaves_of(fragment.node)
                .iter()
                .map(|&leaf| tree.token(leaf))
                .collect();
            similar::signature(&tokens)
        })
        .collect();

    let mut buckets: BTreeMap<(FragmentKind, u64), Vec<usize>> = BTreeMap::new();
    for (index, signature) in signatures.iter().enumerate() {
        for band in similar::bands(signature) {
            buckets
                .entry((eligible[index].kind, band))
                .or_default()
                .push(index);
        }
    }

    let mut pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for members in buckets.values() {
        if members.len() <= BUCKET_PAIR_LIMIT {
            for (offset, &left) in members.iter().enumerate() {
                for &right in &members[offset + 1..] {
                    pairs.insert((left.min(right), left.max(right)));
                }
            }
        } else {
            for window in members.windows(2) {
                pairs.insert((window[0].min(window[1]), window[0].max(window[1])));
            }
        }
    }

    let pairs: Vec<(usize, usize)> = pairs.into_iter().collect();
    let confirmed: Vec<(usize, usize, f64, bool)> = pairs
        .par_iter()
        .filter_map(|&(left, right)| {
            let (a, b) = (eligible[left], eligible[right]);
            let (similarity, edit) =
                confirm(files, a, b, &signatures[left], &signatures[right], limits)?;
            Some((left, right, similarity, edit))
        })
        .collect();

    let mut parent: Vec<usize> = (0..eligible.len()).collect();
    for &(left, right, _, _) in &confirmed {
        let (a, b) = (find(&mut parent, left), find(&mut parent, right));
        if a != b {
            parent[b] = a;
        }
    }

    // Per group: its members, its weakest confirmed pair, and whether edit
    // distance confirmed any of them.
    let mut groups: BTreeMap<usize, (Vec<usize>, f64, bool)> = BTreeMap::new();
    for &(left, right, similarity, edit) in &confirmed {
        let root = find(&mut parent, left);
        let entry = groups.entry(root).or_insert((Vec::new(), 1.0, false));
        entry.0.push(left);
        entry.0.push(right);
        entry.1 = entry.1.min(similarity);
        entry.2 |= edit;
    }

    groups
        .into_values()
        .map(|(mut members, similarity, edit)| {
            members.sort_unstable();
            members.dedup();
            let mut detectors = BTreeSet::from([Detector::MinHash]);
            if edit {
                detectors.insert(Detector::EditDistance);
            }
            let first = eligible[members[0]];
            Candidate {
                units: members
                    .iter()
                    .map(|&member| Unit {
                        file: eligible[member].file,
                        nodes: vec![eligible[member].node],
                    })
                    .collect(),
                fragment: first.kind,
                kind: CloneKind::NearMiss,
                detectors,
                similarity,
                shape: mix(first.shape, hash_str("near")),
                recursive: false,
            }
        })
        .collect()
}

/// Confirms one proposed near-miss pair, returning its similarity and whether
/// tree edit distance decided it.
fn confirm(
    files: &[Parsed<'_>],
    a: &Fragment,
    b: &Fragment,
    a_signature: &[u64; similar::SIGNATURE],
    b_signature: &[u64; similar::SIGNATURE],
    limits: &Limits,
) -> Option<(f64, bool)> {
    let (tree_a, tree_b) = (&files[a.file as usize].tree, &files[b.file as usize].tree);
    let (data_a, data_b) = (
        &tree_a.nodes[a.node as usize],
        &tree_b.nodes[b.node as usize],
    );

    if a.shape == b.shape {
        return None;
    }
    if a.file == b.file && (tree_a.contains(a.node, b.node) || tree_a.contains(b.node, a.node)) {
        return None;
    }

    // The metric vector: sizes alone rule most pairs out.
    let size =
        |left: u32, right: u32| similar::ratio(left.min(right) as usize, left.max(right) as usize);
    let size_a = data_a.end - a.node;
    let size_b = data_b.end - b.node;
    if size(data_a.leaf_count, data_b.leaf_count) < limits.similarity
        || size(size_a, size_b) < limits.similarity
    {
        return None;
    }

    let dice = similar::dice(
        &similar::kind_bag(tree_a, a.node),
        &similar::kind_bag(tree_b, b.node),
    );
    if dice < limits.similarity {
        return None;
    }

    if size_a <= EDIT_DISTANCE_LIMIT && size_b <= EDIT_DISTANCE_LIMIT {
        let distance = similar::edit_distance(tree_a, a.node, tree_b, b.node);
        let similarity = 1.0 - similar::ratio(distance, size_a.max(size_b) as usize);
        return (similarity >= limits.similarity).then_some((similarity, true));
    }

    let jaccard = similar::estimated_jaccard(a_signature, b_signature);
    (jaccard >= limits.similarity).then_some((dice.min(jaccard), false))
}

/// Union-find root with path halving.
fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Deduplicates, de-nests, and subsumes the groups from every layer.
fn merge(files: &[Parsed<'_>], found: Vec<Candidate>) -> Vec<Candidate> {
    let mut by_units: BTreeMap<Vec<Unit>, Candidate> = BTreeMap::new();
    for mut candidate in found {
        tidy(files, &mut candidate);
        if candidate.units.len() < 2 {
            continue;
        }
        match by_units.get_mut(&candidate.units) {
            Some(existing) => {
                existing.detectors.extend(candidate.detectors);
                existing.kind = existing.kind.min(candidate.kind);
                existing.similarity = existing.similarity.max(candidate.similarity);
                existing.recursive |= candidate.recursive;
            }
            None => {
                by_units.insert(candidate.units.clone(), candidate);
            }
        }
    }

    let groups: Vec<Candidate> = by_units.into_values().collect();

    // Index every unit by file, so subsumption looks only at its neighbours.
    let mut by_file: BTreeMap<u32, Vec<(usize, usize)>> = BTreeMap::new();
    for (group, candidate) in groups.iter().enumerate() {
        for (unit, data) in candidate.units.iter().enumerate() {
            by_file.entry(data.file).or_default().push((group, unit));
        }
    }

    let subsumed: BTreeSet<usize> = (0..groups.len())
        .into_par_iter()
        .filter(|&index| {
            let candidate = &groups[index];
            let probe = &candidate.units[0];
            by_file
                .get(&probe.file)
                .into_iter()
                .flatten()
                .filter(|&&(other, unit)| {
                    other != index && groups[other].units[unit].encloses(probe, files)
                })
                .any(|&(other, _)| subsumes(files, &groups[other], other, candidate, index))
        })
        .collect();

    groups
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !subsumed.contains(index))
        .map(|(_, candidate)| candidate)
        .collect()
}

/// Whether group `outer` (at index `outer_index`) makes group `inner`
/// redundant: it has at least as many copies, and each of `inner`'s copies
/// lies inside one of its copies.
///
/// Two groups whose copies span exactly the same bytes — a node and its only
/// child — subsume each other, so the tie goes to the earlier one.
fn subsumes(
    files: &[Parsed<'_>],
    outer: &Candidate,
    outer_index: usize,
    inner: &Candidate,
    inner_index: usize,
) -> bool {
    if outer.units.len() < inner.units.len() {
        return false;
    }
    let mut identical = outer.units.len() == inner.units.len();
    for unit in &inner.units {
        let Some(host) = outer.units.iter().find(|host| host.encloses(unit, files)) else {
            return false;
        };
        identical &= host.span(files) == unit.span(files);
    }
    !identical || outer_index < inner_index
}

/// Sorts a group's units, removes nested copies (marking the group recursive),
/// and removes copies that overlap an earlier one.
fn tidy(files: &[Parsed<'_>], candidate: &mut Candidate) {
    candidate.units.sort_by(|left, right| {
        files[left.file as usize]
            .input
            .path
            .cmp(files[right.file as usize].input.path)
            .then_with(|| left.span(files).0.cmp(&right.span(files).0))
            .then_with(|| right.span(files).1.cmp(&left.span(files).1))
    });

    let mut kept: Vec<Unit> = Vec::with_capacity(candidate.units.len());
    for unit in std::mem::take(&mut candidate.units) {
        if let Some(previous) = kept.last() {
            if previous.encloses(&unit, files) {
                candidate.recursive = true;
                continue;
            }
            if previous.overlaps(&unit, files) {
                continue;
            }
        }
        kept.push(unit);
    }
    candidate.units = kept;
}

/// Narrows an index to `u32`; node and file counts are far below four billion.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod test;
