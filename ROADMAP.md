# Roadmap

What is not built yet, roughly in the order it would be worth building. Nothing
here is committed to; the list exists so that a gap in the tool reads as a known
gap rather than an oversight.

## Next

- **Compare two reports.** The report already serializes and carries a schema
  version, so `tinyanalyzer --compare before.json` is mostly rendering: what got
  heavier, what got lighter, what findings a branch introduced. This is the
  feature that turns the tool from a snapshot into a ratchet.
- **A CI mode.** Exit non-zero when a finding at or above a given severity
  appears, so a repository can hold a line it has reached.
- **Build-time attribution.** `cargo build --timings` knows which crates
  dominate a build. Joining that to the dependency weights this tool already
  computes would answer "what is actually making my builds slow" rather than
  "what is large".

## Later

- **Call-graph-aware dead code.** The identifier census is deliberately cheap and
  documented as approximate. Resolving `use` statements and module paths would
  raise confidence on the medium-confidence half of the list without needing a
  compiler.
- **Per-function ownership of unsafe and panic paths.** Currently counted per
  file; per function would make the table directly actionable.
- **Binary size attribution.** Which crates and which monomorphizations account
  for the shipped binary.
- **More languages.** The line counter already handles a dozen; item-level
  analysis is Rust only, and the module boundary for adding another is
  `rust_source`.

## Deliberately not planned

- **A web dashboard.** The tool ships as one binary with no runtime and nothing
  fetched at startup, and that is a feature rather than a limitation: it runs
  over SSH, in a container, and on a machine with no browser.
- **Auto-fixing.** Every finding names a specific remedy, and every remedy is a
  judgment call about a codebase this tool has not read the history of. Rewriting
  somebody's code on the strength of a heuristic is a different product.
