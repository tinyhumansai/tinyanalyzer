//! Near-miss similarity: `MinHash` signatures, node-kind Dice, and tree edit
//! distance.
//!
//! The three are layered cheapest first. `MinHash` over token shingles, banded
//! into locality-sensitive buckets, proposes pairs without comparing every
//! fragment to every other. A metric vector — leaf and node counts — throws out
//! pairs whose sizes alone rule them out. Dice similarity over the bag of node
//! kinds confirms the rest, and Zhang–Shasha tree edit distance has the final
//! word on fragments small enough to afford it.

use crate::clones::syntax::{Class, Tree, hash_str, mix};
use std::collections::BTreeMap;

/// Hash functions per `MinHash` signature.
pub(crate) const SIGNATURE: usize = 32;
/// Rows per LSH band; `SIGNATURE / BAND_ROWS` bands.
pub(crate) const BAND_ROWS: usize = 4;
/// Tokens per shingle.
const SHINGLE: usize = 5;

/// The `MinHash` signature of a token sequence.
#[must_use]
pub(crate) fn signature(tokens: &[u32]) -> [u64; SIGNATURE] {
    let mut minimum = [u64::MAX; SIGNATURE];
    let width = SHINGLE.min(tokens.len().max(1));
    for window in tokens.windows(width) {
        let base = window
            .iter()
            .fold(0_u64, |hash, &token| mix(hash, u64::from(token)));
        for (seed, slot) in (0_u64..).zip(minimum.iter_mut()) {
            let hashed = mix(base, seed);
            if hashed < *slot {
                *slot = hashed;
            }
        }
    }
    minimum
}

/// The LSH bucket keys of a signature, one per band.
#[must_use]
pub(crate) fn bands(signature: &[u64; SIGNATURE]) -> Vec<u64> {
    signature
        .chunks(BAND_ROWS)
        .zip(0_u64..)
        .map(|(rows, band)| rows.iter().fold(band, |hash, &row| mix(hash, row)))
        .collect()
}

/// The fraction of signature slots two signatures agree on: an unbiased
/// estimate of the Jaccard similarity of their shingle sets.
#[must_use]
pub(crate) fn estimated_jaccard(left: &[u64; SIGNATURE], right: &[u64; SIGNATURE]) -> f64 {
    let agree = left.iter().zip(right).filter(|(a, b)| a == b).count();
    ratio(agree, SIGNATURE)
}

/// The multiset of node kinds in a subtree.
#[must_use]
pub(crate) fn kind_bag(tree: &Tree<'_>, root: u32) -> BTreeMap<&'static str, u32> {
    let mut bag = BTreeMap::new();
    let end = tree.nodes[root as usize].end;
    for node in &tree.nodes[root as usize..end as usize] {
        *bag.entry(node.kind).or_insert(0) += 1;
    }
    bag
}

/// Dice similarity of two multisets: twice the overlap over the total.
#[must_use]
pub(crate) fn dice(left: &BTreeMap<&str, u32>, right: &BTreeMap<&str, u32>) -> f64 {
    let overlap: u32 = left
        .iter()
        .map(|(kind, &count)| count.min(right.get(kind).copied().unwrap_or(0)))
        .sum();
    let total: u32 = left.values().sum::<u32>() + right.values().sum::<u32>();
    ratio(2 * overlap as usize, total as usize)
}

/// `numerator / denominator`, or `1.0` when both are zero.
#[must_use]
pub(crate) fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 1.0;
    }
    // Both are node or token counts, far below `f64`'s exact-integer range.
    #[allow(clippy::cast_precision_loss)]
    {
        numerator as f64 / denominator as f64
    }
}

/// A subtree in postorder, as Zhang–Shasha reads it.
struct Postorder {
    /// Label of each node.
    labels: Vec<u64>,
    /// Postorder index of each node's leftmost leaf.
    leftmost: Vec<usize>,
    /// Nodes with no later node sharing their leftmost leaf.
    keyroots: Vec<usize>,
}

impl Postorder {
    fn new(tree: &Tree<'_>, root: u32) -> Self {
        let mut labels = Vec::new();
        let mut leftmost = Vec::new();
        // Iterative so a deep tree cannot overflow the stack.
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        while let Some((node, expanded)) = stack.pop() {
            let data = &tree.nodes[node as usize];
            if expanded {
                labels.push(if data.class == Class::Inner {
                    hash_str(data.kind)
                } else {
                    data.shape
                });
                // In postorder a subtree's first node is its leftmost leaf.
                let size = (data.end - node) as usize;
                leftmost.push(labels.len() - size);
                continue;
            }
            stack.push((node, true));
            let children: Vec<u32> = tree.children(node).collect();
            stack.extend(children.iter().rev().map(|&child| (child, false)));
        }

        let mut keyroots = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for index in (0..leftmost.len()).rev() {
            if seen.insert(leftmost[index]) {
                keyroots.push(index);
            }
        }
        keyroots.reverse();

        Self {
            labels,
            leftmost,
            keyroots,
        }
    }
}

/// Zhang–Shasha tree edit distance with unit costs.
///
/// `O(n · m · min(depth, leaves)²)`, so callers bound the subtree sizes first.
#[must_use]
pub(crate) fn edit_distance(
    left_tree: &Tree<'_>,
    left: u32,
    right_tree: &Tree<'_>,
    right: u32,
) -> usize {
    let a = Postorder::new(left_tree, left);
    let b = Postorder::new(right_tree, right);
    let (n, m) = (a.labels.len(), b.labels.len());
    let mut distance = vec![0_usize; n * m];
    let mut forest: Vec<usize> = Vec::new();

    for &i in &a.keyroots {
        for &j in &b.keyroots {
            let (li, lj) = (a.leftmost[i], b.leftmost[j]);
            let rows = i - li + 2;
            let cols = j - lj + 2;
            forest.clear();
            forest.resize(rows * cols, 0);
            for x in 1..rows {
                forest[x * cols] = forest[(x - 1) * cols] + 1;
            }
            for y in 1..cols {
                forest[y] = forest[y - 1] + 1;
            }
            for x in 1..rows {
                let i1 = li + x - 1;
                for y in 1..cols {
                    let j1 = lj + y - 1;
                    let delete = forest[(x - 1) * cols + y] + 1;
                    let insert = forest[x * cols + y - 1] + 1;
                    let value = if a.leftmost[i1] == li && b.leftmost[j1] == lj {
                        let rename = usize::from(a.labels[i1] != b.labels[j1]);
                        let value = delete.min(insert).min(forest[(x - 1) * cols + y - 1] + rename);
                        distance[i1 * m + j1] = value;
                        value
                    } else {
                        let px = a.leftmost[i1] - li;
                        let py = b.leftmost[j1] - lj;
                        delete
                            .min(insert)
                            .min(forest[px * cols + py] + distance[i1 * m + j1])
                    };
                    forest[x * cols + y] = value;
                }
            }
        }
    }

    distance[n * m - 1]
}
