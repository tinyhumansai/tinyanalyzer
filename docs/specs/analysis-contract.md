# Specification: the analysis contract

**Status:** accepted
**Applies to:** `crates/tinyanalyzer-core`

What `tinyanalyzer` promises about its own output. This is the document to read
before trusting a number, changing a measurement, or adding a rule.

## The promise

Every number in a report is one of three things, and the report always says
which:

1. **Exact** — a direct count of something on disk or in the syntax tree.
2. **A documented approximation** — a heuristic whose failure modes are stated,
   along with which direction it fails in.
3. **Absent** — because the input could not be understood.

There is no fourth category. In particular there is no "best guess presented as
a fact", which is the failure mode that makes a tool like this worse than
nothing: a wrong number that looks exact costs more than a missing one, because
somebody acts on it.

## What is exact

- **Line counts.** Every line is classified exactly once as code, comment, or
  blank; `code + comment + blank == total` is asserted in the test suite. A line
  carrying both code and a trailing comment counts as code, because it has to be
  read to understand the program.
- **Item counts, function lengths, parameter counts, nesting depth.** From a
  real parse, not a regular expression. A `fn` inside a doc comment is not a
  function.
- **The dependency graph.** From `cargo metadata` — cargo's own resolution,
  including features, optional dependencies, platform-specific edges, and
  version unification. A tool that re-implemented any of those would disagree
  with the build it is describing.
- **Dependency source size.** The bytes in each resolved external package's
  checked-out source directory, excluding `.git` and `target`. This measures the
  source Cargo consumes; it does not predict target-, profile-, feature-, or
  LTO-dependent linked binary size.
- **Exclusive dependency weight.** How many packages become unreachable from the
  workspace if one direct dependency is removed. Computed by cutting every edge
  into that package and re-walking, which is exact for the graph cargo returned.

## What is approximate, and in which direction

### Cyclomatic complexity

Counts branches, not paths: one, plus one each for `if`, a `match` arm, `while`,
`for`, `loop`, `&&`, `||`, and `?`.

A `match` arm whose body names a constant — a literal, a path, or a small call
over those — is **not** counted, unless it carries a guard. A dispatch table is
not a decision, and counting it would make every exhaustive `match` over an enum
rank as the worst function in a well-typed codebase, which trains a reader to
ignore the metric entirely.

A nested function is scored on its own and does not contribute to its parent.

The number is meaningless in isolation and meaningful in a ranking. That is the
only way it is used.

### Dead code

A workspace-wide identifier census over the token stream. Every file contributes
the count of each identifier it contains; every definition contributes one
occurrence of its own name at its declaration. A name seen no more often than it
is declared is referenced by nothing.

**Why tokens rather than name resolution:** resolving names properly requires
being a compiler. The census runs in well under a second on a large repository
and — crucially — sees inside macro invocations, which the syntax tree stores as
an opaque blob. A helper called only from a `macro_rules!` body is invisible to
an AST walk and perfectly visible here.

**Which direction it fails:** it **under-reports**. Two unrelated items sharing a
name vouch for each other. That is the correct direction for a list a human is
going to act on by deleting things.

**What it never reports:**

- Modules. A `mod` declaration is a namespace, not a symbol: its contents compile
  whether or not anything names the module, so "unreferenced module" would be
  true of almost every module in a well-organized crate and would mean nothing.
- Trait and inherent methods, which are reached through their trait or type.
- Anything carrying `#[no_mangle]`, `#[export_name]`, `#[used]`, a proc-macro
  attribute, `#[macro_export]`, or `#[test]` — names meaningful to something
  other than Rust, whose caller the census cannot see.
- Anything in the configured `ignore` list.

**Confidence** is reported per item. A private item comes back `high`: every
possible caller was in scope. A `pub` item comes back `medium`: a library's
callers may not be in this repository at all.

By default, a reference from test code does **not** count as a use. An item only
its own tests call is dead weight in the shipped binary, which is exactly the
question this tool exists to answer. `tests_count_as_uses = true` reverses that.

### Unused dependencies

A declared dependency that no source file in the declaring package *names*, with
hyphens folded to underscores. A crate reached only through a derive macro, a
build script, or a linker side effect has no `use` naming it.

The remedy in the finding says so: remove it and build. If the build passes it
was costing compile time for nothing; if it fails, the crate belongs in
`ignore_unused`.

No conclusion is drawn about a package whose source files were not analyzed —
silence beats a page of false positives.

### File weight

A single number used only to order the file list. Lines of code, plus three per
unit of branching beyond the one path every function has, plus five per level of
nesting beyond three, plus ten per allocation inside a loop.

The weights are chosen so a file has to be substantially more complex, not
marginally so, to outrank a file twice its length. The number has no meaning
outside the ranking and is not reported as if it did.

## What is absent, and why it is said out loud

A Rust file the parser refuses produces **no** parsed measurements — not partial
ones — and appears in `parse_failures` and as a `parse_failure` finding.

This matters more than it looks. A file silently dropped from the item,
complexity, and dead-code passes still appears in the line counts, so it reads
as a clean file rather than an unknown one. The finding says explicitly that the
file is counted in the line totals and absent from everything else.

A file too large to read, or one that is not UTF-8, is reported with its byte
count and no contents, for the same reason.

A workspace `cargo metadata` cannot resolve leaves the dependency graph empty
rather than failing the whole analysis. A tree mid-refactor is a normal state,
and the file-level half of the report — which is most of it — is still worth
having.

## Determinism

Two runs over an unchanged tree produce byte-identical reports. Files are sorted
before ranking, ties break on path, and every aggregation is over a sorted map.

This is not cosmetic. A report that reorders itself run to run cannot be diffed,
and diffing two reports is how you see what a refactor actually did.

## Rules

A rule turns a measurement into advice. Every rule:

- compares against a threshold from `Thresholds`, never a constant in the rule —
  with one documented exception, `DEEP_NESTING`, where the remedy is the same
  regardless of house style;
- states what was measured, with the numbers in it;
- states what to do about it.

`Rule` identifiers are part of the serialized report format. Renaming one is a
breaking change to the schema.

Findings are ranked by severity, then by the measurement behind them — so the
top of the list is the thing most worth fixing, not the first thing the walker
happened to see.

## The report format

`schema_version` moves when a field is removed or changes meaning. Adding a
field does not move it. The whole report round-trips through JSON, which is
asserted in the integration suite, because anything storing or diffing a report
depends on it.
