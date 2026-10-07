# `clones` — duplicate code and the symbol index

Finds code written more than once and sketches what would replace it. The
behavior is specified in
[`docs/specs/clone-detection.md`](../../../../docs/specs/clone-detection.md);
this file is about how it is built.

## Pipeline

```text
CloneInput (path, text, test?, editable?)
  │  syntax::parse — tree-sitter, flattened to a preorder arena
  ▼
Tree ── fragments ──► detect::run ──► Candidate groups
  │                    ├ subtree hashes   (whole fragments, equal shapes)
  │                    ├ suffix array     (repeated token runs, trimmed)
  │                    └ near misses      (MinHash → size → Dice → edit distance)
  │                    merge: dedupe, de-nest, subsume
  ▼
build (filter, score) ──► sketch::sketch (anti-unification) ──► CloneGroup
  │
  └─► symbols::index ──► SymbolRecord
```

## Modules

| Module | Holds |
|---|---|
| `syntax/` | Parsing, the flattened `Tree`, leaf classes, the structural hash, tree navigation |
| `detect/` | The three detection layers and the merge; `suffix.rs` and `similar.rs` hold the algorithms |
| `sketch/` | Aligning copies, turning differences into parameters, choosing the kind of shared code |
| `symbols/` | The per-item index |
| `types.rs` | Everything public: groups, instances, sketches, parameters, symbol records |

## Design notes

**One flattened tree.** Each file becomes a `Vec<Node>` in preorder. A node's
subtree is the index range `index..end`, so containment is two comparisons and
walking children is following `end`. Leaves are listed separately in source
order, and a subtree's leaves are a contiguous slice of that list — which is
what lets the suffix array's token positions map straight back to syntax.

**Normalization is the leaf class.** Identifiers and type names hash the same;
literals hash by kind; keywords and punctuation by their text. The structural
hash also sorts adjacent statements in a block that share no variable name, so
copies with reordered independent statements match.

**Exact, renamed, near-miss.** A group of equal shapes is exact when the exact
text of every leaf matches, renamed otherwise, and near-miss when the token
sequences differ (only reordering can do that with equal shapes). The
near-miss layer's groups are near-misses by construction.

**Type shapes keep type names.** For structs and enums the hash keeps the field
types and blinds every name, so two structs with the same field types in the
same order match whatever they are called.

**Merging prefers the largest thing.** Groups found by several layers are
merged by their exact set of copies. A group whose copies all sit inside the
copies of another group with at least as many copies is dropped, and after
ranking, a group most of whose copies overlap a better group is dropped too, so
the suffix array's many cuts of one repeated sequence collapse to the best one.

**Sketches are anti-unification.** The first copy is aligned against each other
copy. Equal-length sibling runs align by position; otherwise a weighted longest
common subsequence (exact matches count double) pairs what it can, unpaired
siblings in the first copy are missing elsewhere, and unpaired siblings in the
other copy are insertions. The outermost differing node on each path becomes a
parameter, and one renamed identifier is one parameter however often it
appears.

## Costs and limits

- Every file is parsed a second time, by tree-sitter. On a 115,000-line
  repository the whole pass takes about a second and a quarter of a gigabyte.
- Tree edit distance is only run on fragments of up to 300 nodes
  (`EDIT_DISTANCE_LIMIT`); larger ones are confirmed by Dice and `MinHash`.
- A repeated run keeps at most 64 occurrences (`MAX_OCCURRENCES`), and an LSH
  bucket with more than 32 members is chained rather than fully paired
  (`BUCKET_PAIR_LIMIT`), so one boilerplate line cannot make the pass quadratic.
- Macro bodies are token trees, not syntax: the suffix array still sees their
  tokens, but no fragment inside one is recognized.
