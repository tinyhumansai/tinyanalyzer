# Specification: clone detection and the symbol index

**Status:** implemented
**Applies to:** `crates/tinyanalyzer-core/src/clones/`, the `duplicate_code`
rule, `--output symbols`, and the dashboard's Duplicates view
**Decision record:** [ADR 2](../adr/0002-tree-sitter-for-structural-analysis.md)
**Plan:** [`../plans/clone-detection.md`](../plans/clone-detection.md)

## Problem

A codebase that has grown by copying carries the same logic in several places.
Each copy costs reading time, and a fix applied to one copy and not the others
is a bug. Finding the copies by eye does not scale past a few files. A team that
wants to shrink its code needs a ranked list of what is repeated, and what
shared code would replace each repetition.

## Goals and non-goals

Goals:

- Find code written more than once at every scale: a run of statements inside a
  function, a match arm, a closure, a whole function or impl block, a struct or
  enum definition.
- Find exact copies, renamed copies, and near-misses (copies with statements
  added, removed, reordered, or changed).
- Rank the groups by how much code folding them would remove.
- For each group, sketch what would replace it: a function, a generic, a macro,
  a loop over a table, a recursive helper, or a shared type, with the
  differences between the copies as its parameters.
- Index every item as a record of its signature, shape, and names, for tools
  that want to look for reuse in ways the groups do not cover.
- Let the operator scan crates outside the analysis root and mark code that no
  suggestion may ask to change.

Non-goals:

- Rewriting code. The sketch is advice, not a patch.
- Semantic ("type 4") clones: code that does the same thing written
  differently. That needs program dependence graphs or learned embeddings,
  which are too slow for an interactive tool or need models this binary does
  not carry.
- Full program-dependence-graph matching. Only adjacent independent statements
  are allowed to reorder.

## Proposed behavior

`analyze_with` runs clone detection after dead code whenever
`clones.enabled` is set (the default) and fills `Report::clones`, a ranked
`Vec<CloneGroup>`. Each group has the copies (`CloneInstance`: file, lines,
enclosing item, editable, test), how alike they are (`CloneKind`: exact,
renamed, near-miss), the granularity (`FragmentKind`), which detectors found
it, the lowest pairwise similarity, size, estimated lines saved, a score, and a
`Sketch`.

Detection layers, cheapest first:

1. **Subtree hashes**: whole fragments whose identifier-blind shapes match.
   Independent adjacent statements are hashed in canonical order.
2. **Suffix array**: maximal repeated runs of normalized tokens (the
   `CCFinder` method), trimmed to whole statements or items.
3. **Near-miss**: `MinHash` and LSH over token shingles propose pairs; a size
   check (the metric vector) and node-kind Dice similarity filter them; Zhang–Shasha
   tree edit distance confirms fragments of up to 300 nodes.

Groups found by several layers are merged. A copy nested in another copy of the
same group is dropped and the group marked recursive. A group whose copies all
sit inside the copies of a group with at least as many copies is dropped. In
score order, a group most of whose copies overlap a better group is dropped.

Score is `(copies − 1) × tokens × similarity`, halved when every copy is test
code and halved when any copy is read-only. Groups with no editable copy are
not reported. Lines saved is `(copies − 1) × lines − copies`: every copy but one
goes, and each leaves a call behind.

`duplicate_code` findings are raised for groups that have an editable copy, are
not test-only, and save at least `duplicate_min_lines` lines. They are high
severity when they save at least `long_function_lines`.

`tinyanalyzer --output symbols` writes one `SymbolRecord` per item as JSON
lines, and runs nothing else.

Configuration:

```toml
[thresholds]
duplicate_min_tokens = 50    # shortest copy, in normalized tokens
duplicate_min_lines = 6      # shortest copy, in lines; also the finding cut-off
duplicate_similarity = 0.85  # lowest near-miss similarity

[clones]
enabled = true
read_only = ["vendor/**"]    # indexed, never asked to change
fragment_kinds = ["function", "impl", "trait", "closure", "match_arm",
                  "if_chain", "block", "statements", "type_shape"]
include_tests = true
max_groups = 500

[[clones.extra_roots]]       # scanned for clones, not otherwise analyzed
path = "vendor/tinytools"
editable = false
```

## Invariants and constraints

- Two runs over an unchanged tree produce identical groups in identical order.
- Every threshold comes from `Thresholds`. Algorithm cost bounds (the
  tree-edit-distance node limit, the occurrences kept per repeated run, the LSH
  bucket pairing limit) are constants documented where they are defined.
- The engine gains no terminal UI, argument parser, server, or async runtime.
- A file with syntax errors does not abort the analysis; fragments containing
  an error node are not reported.
- Type definitions are held to `duplicate_min_lines` alone, because a struct is
  short in tokens by nature.

## Acceptance criteria

- Two copies of a function in different files, renamed, produce one group of
  kind `renamed` and nothing smaller inside them.
- A shared run of statements inside two otherwise different functions produces
  a `statements` group found by the suffix array.
- A copy with one extra statement is a `near_miss` group, and its sketch has a
  `statements` parameter.
- Adjacent copies differing only in values sketch as a loop.
- A group of read-only copies is not reported; a mixed group is.
- `--no-clones` leaves `clones` empty; `--output symbols` prints one JSON object
  per item.
- On a 115,000-line repository the pass finishes within ten seconds.

## Open questions

None blocking. More grammars, and a rule-suppression list for groups a team has
decided to keep, are natural follow-ups.
