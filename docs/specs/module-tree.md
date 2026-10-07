# Specification: the module tree — orphaned files and test-only files

**Status:** implemented
**Applies to:** `crates/tinyanalyzer-core/src/module_tree/`, the `orphan_file`
rule, `DefinitionKind::File`, and the default `scan.test_patterns`

## Problem

Two classes of file were misreported because the analyzer looked at files one
at a time and never asked which `mod` declaration loads them.

1. **Test files with an ordinary name.** The sibling-file convention
   `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;` puts unit tests in a file
   no default path glob matched. Its contents carry no `#[cfg(test)]` of their
   own, so the AST check missed it too. On one large repository 1,365 clone
   instances in `*_tests.rs` files were counted as production code, test
   helpers flooded the dead-code list, and production items used only by tests
   were hidden because those files fed the census.
2. **Orphaned files.** A `.rs` file no `mod` declaration reaches is never
   compiled. It reads exactly like live code, and a stale copy of a live file is,
   to the clone detector, an exact clone: one repository reported an 805-line
   "exact clone" whose second copy was a file nothing declared.

## Goals and non-goals

Goals:

- `**/*_tests.rs` is a default test glob, beside the existing `**/*_test.rs`.
- A file reached only through `#[cfg(test)]` declarations or test targets is
  test code, whole, whatever its name.
- A file under a package's `src/` that no target root reaches is reported as an
  orphan, as dead code and as an `orphan_file` finding.
- Orphans never appear in clone groups.

Non-goals:

- Evaluating `cfg` predicates. Every declaration is followed whatever its
  `cfg`, except that `#[cfg(test)]` decides test-ness.
- Expanding macros. A declaration a macro generates is approximated (below).
- Orphans outside `src/`. Each top-level file of `tests/`, `examples/`, and
  `benches/` is its own target root.

## Proposed behavior

### Roots

For every package manifest the walk found, the roots are: `src/lib.rs`,
`src/main.rs`, `build.rs`, `src/bin/*.rs`, `src/bin/*/main.rs`, the same two
shapes under `examples/`, `tests/`, and `benches/`, `package.build`, `[lib]
path`, and every `path` of `[[bin]]`, `[[example]]`, `[[test]]`, and
`[[bench]]`. `autobins` and its relatives are ignored: an extra root can only
hide an orphan. Test and bench roots are test roots; a file that is both kinds
of root is production.

### Following declarations

From each root, every `mod name;` is followed the way rustc resolves it:

- A crate root or `mod.rs` resolves children in its own directory; any other
  file `dir/x.rs` resolves them in `dir/x/`. Both `name.rs` and `name/mod.rs`
  are candidates.
- An inline `mod a { mod b; }` adds `a/` (or the inline module's own `#[path]`)
  to the directory.
- `#[path = "..."]` outside an inline module is relative to the declaring file's
  directory; inside one, to the inline module's directory. A file loaded through
  `#[path]` resolves its own children both as a `mod.rs` and under its stem.
- `#[cfg_attr(..., path = "...")]` adds its path without removing the default
  location.
- `mod name;` sequences inside item-position macro bodies (`cfg_if!`) are
  declarations.
- `include!("literal")` reaches the file and follows its declarations as the
  including module's; `include_str!` and `include_bytes!` with a literal only
  mark their file reached.

### Test-only files

A file is test-only when the traversal from every root reaches it, and the
traversal from production roots that skips `#[cfg(test)]` declarations (on the
item, on an enclosing inline module, or `#![cfg(test)]` on the declaring file)
does not. Test-only files get `is_test = true` with all their lines counted as
test lines, are left out of the dead-code census by default, and are passed to
clone detection as test code.

### Orphans

A file under `<package>/src/` whose nearest package manifest is that package's,
that no traversal reached, is an orphan. Each one is:

- a `DeadCodeCandidate` with kind `file`, named after the module it would be,
  at line 1, `high` confidence, `is_test` from the file;
- an `orphan_file` finding at `high` severity — the same as `dead_code` — whose
  metric is the file's lines of code;
- left out of the dead-code census, so nothing in it vouches for anything and
  its items are not listed one by one;
- left out of clone detection. An orphaned copy is not a second copy of
  anything; it is a file to delete, and `orphan_file` says so. Excluding it,
  rather than labeling its instances, keeps clone groups describing compiled
  code only.

The `dead_code` finding counts item candidates only. Turning dead code off
(`dead_code.enabled = false` or `--no-dead-code`) also turns orphan reporting
off; test-only classification still applies.

## Invariants and constraints

Being wrong about an orphan invites a deletion, so the resolver refuses to
guess. A package is **not searched for orphans** when:

- some reachable file contains an `include!` whose argument is not a string
  literal;
- some reachable file failed to parse, was too large to read, or exists on disk
  but was excluded from the walk;
- no root of the package exists.

And a file is **not reported** when:

- its module name appears as an identifier inside an item-position macro
  invocation anywhere the package reaches — the macro may expand to
  `mod name;`;
- it has no items.

`Report` gains no fields: orphans travel in `dead_code` and `findings`, so the
schema version does not move. `DefinitionKind::File` and `Rule::OrphanFile` are
additive variants of `#[non_exhaustive]` enums.

The public `symbol_index` does not parse, so it does not resolve module trees:
it still indexes orphaned files and classifies tests by path and attributes.

## Acceptance criteria

- `src/foo_tests.rs` is test code by path, and a file declared only as
  `#[cfg(test)] #[path = "checks.rs"] mod checks;` is test code by declaration.
- An undeclared file in `src/` produces a `file` dead-code candidate and an
  `orphan_file` finding, and appears in no clone group.
- A package with a computed `include!`, an unparsable reachable file, or a
  macro naming the file produces no orphan.
- On `tinyagents`, `crates/tinyagents-session/src/run_ledger/ops/team.rs` and
  `ops/rows.rs` are reported as orphans and the 805-line `ops.rs` / `team.rs`
  exact clone is gone.

## Open questions

None blocking. A declaration generated by a `macro_rules!` whose module name is
built with `paste!` or `concat_idents!` is not recognized; the identifier
suppression above is the only defense, and it does not cover a name assembled
from pieces.
