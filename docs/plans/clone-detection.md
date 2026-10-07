# Plan: clone detection and the symbol index

**Specification:** [`../specs/clone-detection.md`](../specs/clone-detection.md)
**Status:** complete

## Goal

Implement the specification in test-first steps, each of which builds and
passes the four contract commands on its own.

## Tasks

1. **Dependencies.** Add `tree-sitter` and `tree-sitter-rust` to
   `[workspace.dependencies]` with a comment, and take them in
   `crates/tinyanalyzer-core/Cargo.toml`. Record the decision in
   `docs/adr/0002-tree-sitter-for-structural-analysis.md`.
2. **Syntax layer** — `clones/syntax/`. Tests first: comments and attributes
   dropped, literals one leaf, leaf classes, renamed copies share a shape,
   independent statements reorder and dependent ones do not, test attributes
   propagate, syntax errors are flagged, an empty file is one node. Then the
   flattened preorder `Tree` and its navigation helpers.
3. **Suffix array** — `clones/detect/suffix.rs`. Tests: matches a brute-force
   sort, LCP measures adjacent suffixes, repeats are left-maximal, below the
   minimum is nothing, occurrences are capped.
4. **Detection and merge** — `clones/detect/`. Tests: fragment recognition,
   trimming, renamed and exact functions, reordering, statement runs, near
   misses with and without edit distance, unrelated code, threshold
   boundaries, type shapes, syntax errors, recursion, overlap, merge and
   subsumption.
5. **Sketch** — `clones/sketch/`. Tests: no parameters, typed literals,
   consistent renaming, generic, macro, inserted statements, loops, recursion,
   shared types, naming.
6. **Symbols** — `clones/symbols/`. Tests: every item in order, signature and
   names, shape hashes.
7. **Pipeline** — `clones/mod.rs`. Tests: locations, read-only, tests ranking
   and exclusion, ranking and cap, disabled kinds, JSON lines.
8. **Configuration, report, rule** — `config/types.rs`, `report/`,
   `findings/`. Tests: defaults, parsing, unknown keys, report fields, extra
   roots, read-only globs, invalid glob, symbol index, the `duplicate_code`
   boundary and severity. Pin the serialized form in `tests/public_api.rs`.
9. **Binary** — `cli/`, `main.rs`, `summary/`, `dashboard/`. Tests: flags,
   output mode, summary section, Duplicates view (draw, filter, sort, empty),
   and the built binary's JSON and symbols output.
10. **Documentation** — the analysis contract, `clones/README.md`, the root
    README, and the worked example in `tinyanalyzer.toml`.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
bash .github/scripts/check-file-coverage.sh 90
cargo run -p tinyanalyzer -- . --output summary
```

## Checklist

- [x] Tasks 1–10 landed with their tests.
- [x] Contract commands pass.
- [x] Every source file at or above 90% line coverage.
- [x] Run against this repository and a 115,000-line one.
