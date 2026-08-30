# tinyanalyzer-core

The analysis engine behind [`tinyanalyzer`](../tinyanalyzer). Point it at a
directory and it produces a `Report`.

## Why this is its own crate

Because the analysis is the part worth embedding in something else — a CI check,
an editor plugin, a script that diffs two reports across a refactor — and none
of those want a terminal UI, an argument parser, or an event loop linked into
them.

So this crate has none. No `ratatui`, no `crossterm`, no `clap`, no server, no
async runtime. Its dependencies are a parser, a walker, a glob matcher, a
serializer, and `cargo metadata`. CI asserts it stays that way, because a
dependency on a renderer arrives transitively through a feature somebody enabled
one crate away and is invisible in the diff that introduces it.

The direction matters too. `tinyanalyzer` depends on this crate and re-exports
all of it, so `tinyanalyzer::Report` and `tinyanalyzer_core::Report` are the
*same type*. A parallel set of report types for embedders would mean a
conversion at every call site that nothing checks.

## The pipeline

`report::analyze` runs these in order. Each is usable on its own.

| Module | What it answers |
|---|---|
| `config` | What did the operator ask for? |
| `walk` | Which files are in scope? |
| `loc` | How much of each file is code, comment, and blank? |
| `rust_source` | What does each Rust file define, and how tangled is it? |
| `deps` | What does the dependency graph actually cost? |
| `dead_code` | What does nothing reference? |
| `findings` | What should somebody do about all this? |
| `report` | All of it, joined and ranked. |

Parsing is parallel because it dominates wall-clock time and each file is
independent. Aggregation is sequential because it is cheap and order-sensitive,
and sequential is where it is easy to be sure it is right.

## What it approximates

Each module documents its own trade-offs; these are the three that change how you
should read the output.

**Cyclomatic complexity** (`rust_source`) counts branches rather than paths, and
deliberately does not count a `match` arm that names a constant — a dispatch
table is not a decision, and counting it would make every exhaustive `match` in a
well-typed codebase rank as its worst function.

**Dead code** (`dead_code`) is an identifier census over the token stream, not
name resolution. That is what lets it see through macro invocations, which an AST
walk cannot. The cost is that two unrelated items sharing a name vouch for each
other, so it under-reports rather than over-reports — the right direction for a
list a human will act on. Public items come back at medium confidence because
their callers may not be in this workspace at all.

**Unused dependencies** (`deps`) are dependencies no source file *names*. A crate
reached only through a derive macro or a linker side effect has no `use` naming
it, which is what `ignore_unused` is for.

**Everything else** — line counts, item counts, function lengths, nesting, the
dependency graph — is exact. The graph in particular comes from `cargo metadata`
rather than from re-parsing manifests, because features, optional dependencies,
platform-specific edges, and version unification are decided by the resolver, and
a tool that re-implements any of them will disagree with the build it is
describing.

## Example

```rust
use tinyanalyzer_core::{Config, Report, analyze_with};

let mut config = Config::default();
config.dependencies.enabled = false;   // skip cargo; pure filesystem work

let report: Report = analyze_with(".", &config)?;

println!("{} files, {} lines", report.totals.files, report.totals.lines.code);

for file in report.files.iter().take(5) {
    println!("{:<50} {} loc", file.path, file.lines.code);
}
# Ok::<(), tinyanalyzer_core::Error>(())
```
