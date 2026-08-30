# Repository Guidelines

This file is the single source of truth for how humans and coding agents work
in this repository. `CLAUDE.md` is a symlink to this file, so every agent reads
the same instructions.

## What This Is

`tinyanalyzer` reads a Rust repository and tells you where its weight, its cost,
and its dead code are — then opens a terminal dashboard over the answer. It
ships as one binary with no runtime, no browser, and nothing fetched at startup.

Two rules follow from that and are worth stating before anything else, because
both are asserted in CI rather than merely encouraged:

1. **The engine stays free of the interface.** `crates/tinyanalyzer-core` must
   never depend on a terminal UI, an argument parser, a server, or an async
   runtime. It is the crate another tool embeds.
2. **The binary carries its own interface.** Nothing in `crates/` may reference
   an external asset host. A dashboard that needed a CDN would fail in exactly
   the environment this tool is for.

## Project Structure

This is a Rust 2024 cargo workspace rooted at a virtual `Cargo.toml`. Every
crate lives under `crates/`, one directory per package, each directory named for
the package it holds. There is no root package: the crate that ships as the
executable is `crates/tinyanalyzer`, the same as any other member.

```text
Cargo.toml              # virtual workspace: members, [workspace.package],
                        # [workspace.dependencies], [workspace.lints]
tinyanalyzer.toml       # this tool's configuration for this repository, which
                        # is also the worked example of the format
crates/
├── tinyanalyzer-core/  # the analysis engine: measurements, no interface
│   ├── README.md       # why the engine is its own crate
│   └── src/
│       ├── lib.rs      # crate docs + the entire public re-export surface
│       ├── error/      # crate-wide `Error` and `Result<T>`
│       ├── config/     # `tinyanalyzer.toml`
│       ├── walk/       # which files are in scope
│       ├── loc/        # code, comment, and blank lines
│       ├── rust_source/# syn-based items, complexity, and cost signals
│       ├── deps/       # the resolved dependency graph and its real cost
│       ├── dead_code/  # the workspace-wide identifier census
│       ├── findings/   # the rules that turn measurements into advice
│       └── report/     # all of it, joined and ranked
└── tinyanalyzer/       # the binary: command line, text output, dashboard
    ├── src/
    │   ├── main.rs     # argument parsing and the exit code, nothing else
    │   ├── lib.rs      # crate docs + public surface, re-exporting the engine
    │   ├── error/      # crate-wide `Error` and `Result<T>`
    │   ├── cli/        # the command line and the configuration it produces
    │   ├── summary/    # the non-interactive text report
    │   └── dashboard/  # state machine, renderer, event loop
    ├── tests/          # integration tests against the built binary
    └── examples/       # runnable, compiled-in-CI usage examples
docs/
├── specs/              # behavior and architecture specifications
├── plans/              # test-first implementation plans
└── adr/                # immutable architecture decision records
```

### The two-crate split

`crates/tinyanalyzer-core` holds every measurement and every rule. It has no
terminal, no renderer, no argument parser, and no runtime, and CI asserts it
stays that way. A tool embedding the analysis depends on it alone.

`crates/tinyanalyzer` depends on it and re-exports all of it, so
`tinyanalyzer::Report` and `tinyanalyzer_core::Report` are the *same* type
rather than structural twins. That direction is load-bearing: a parallel set of
report types for embedders would mean a conversion at every call site that
nothing checks.

The rule for deciding where something goes: anything that measures a repository
or decides what a measurement means belongs in the engine; anything that reads a
key, draws a cell, parses a flag, or writes to a terminal belongs in the binary.

### Where new analysis goes

A new measurement is a new module directory in the engine, not a new function in
an existing one. A new piece of *advice* about an existing measurement is a new
`Rule` variant and a new rule function in `findings/`, and:

- every threshold it compares against comes from `Thresholds`, never a constant
  in the rule — a rule a team cannot disagree with is a rule they stop reading;
- every finding states what was measured, with the numbers in it;
- every finding states what to do about it.

`Rule` variants are part of the serialized report format. Renaming one is a
breaking change to the schema, not a refactor.

Add a crate by creating `crates/<name>/` — `members = ["crates/*"]` picks it up
by existing. Inherit `version`, `edition`, `rust-version`, `license`, and
`repository` from `[workspace.package]`, take shared dependencies from
`[workspace.dependencies]`, and opt into the shared lint set with:

```toml
[lints]
workspace = true
```

Each feature area belongs in a focused module directory under a crate's `src/`.
A module root explains the module, wires its pieces together, and exposes the
smallest useful API. Move substantial type definitions into `types.rs` and put
module-local unit tests in a dedicated `test.rs`, wired from the bottom of the
module root with:

```rust
#[cfg(test)]
mod test;
```

Do not accumulate inline `mod tests` blocks in implementation files, and do not
let a general-purpose `utils.rs` or `helpers.rs` grow — those are a symptom of a
missing module. Prefer many small modules that each do one thing well over few
broad ones.

Keep public exports centralized in each crate's `src/lib.rs` so downstream users
have one predictable surface. Put shared error variants in each crate's
`src/error/mod.rs` and return the crate-wide `Result<T>` from fallible public
APIs.

## Build And Test

Run every command from the repository root. These four are the contract; CI
runs exactly them, so a green local run should mean a green CI run.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
```

Supporting commands:

- `cargo fmt --all` — format before committing.
- `cargo test <filter>` — run a focused subset while iterating.
- `cargo test -p tinyanalyzer-core` — run one crate's suite.
- `cargo run -p tinyanalyzer --example summarize` — run the bundled example.
- `cargo run -p tinyanalyzer -- . --output summary` — run the tool on itself.
  Do this after any change to the engine: the fastest way to find a bug in an
  analyzer is to point it at a real repository, and this one is to hand.
- `cargo doc --no-deps --all-features` — build the rustdoc CI also builds with
  `RUSTDOCFLAGS="-D warnings"`.
- `cargo test --doc` — run doctests alone when editing documentation examples.

Never skip, ignore, or delete a failing test to make a command pass. Fix the
root cause, or stop and report the blocker.

## Coding Style

Use standard `rustfmt` output and Rust 2024 idioms. Do not hand-format around
`rustfmt`, and do not add `#[rustfmt::skip]` without a comment explaining why.

- `snake_case` for modules, files, functions, methods, fields, and locals.
- `PascalCase` for types, traits, and enum variants; `SCREAMING_SNAKE_CASE` for
  constants and statics.
- Name things for what they are, not for their layer: `RetryPolicy`, not
  `RetryHelper`.
- Prefer small, typed APIs over stringly-typed ones. Accept `&str` and generic
  `impl Into<String>` at boundaries; return owned, concrete types.
- Keep the public surface minimal: default to private, and export deliberately
  from `src/lib.rs`.
- `unsafe` is forbidden workspace-wide by `[workspace.lints]` in the root
  `Cargo.toml`. If a project genuinely needs it, relax the lint in its own
  commit and document every invariant with a `// SAFETY:` comment.

### Errors

- One crate-wide `Error` enum per crate, in `src/error/mod.rs`, built with
  `thiserror`.
- Fallible public functions return `Result<T>`, the crate alias.
- Add a specific variant instead of stuffing context into a string; error
  messages are lowercase, without trailing punctuation.
- Do not `unwrap()`, `expect()`, or `panic!` in library code paths. They are
  fine in tests, examples, and genuinely unreachable states — where `expect`
  must carry a message explaining the invariant.
- Document a `# Errors` section on every public fallible function and a
  `# Panics` section on anything that can panic.

### Dependencies

Adding a dependency is a design decision. Before adding one, check whether the
standard library or an existing dependency already covers the need. When you do
add one:

- pin a caret range (`serde = "1"`), not an exact version;
- enable only the features you need, with `default-features = false` when that
  meaningfully trims the tree;
- gate anything optional behind a Cargo feature, documented in `Cargo.toml`;
- declare it once in the root `[workspace.dependencies]` when more than one
  crate needs it, and take it with `{ workspace = true }`;
- never add one to `crates/tinyanalyzer-core` that pulls in a terminal UI, an
  argument parser, a server, or an async runtime — CI fails the build if you do;
- leave a comment above the entry explaining *why* the crate is needed and what
  uses it — see the existing entries for the expected tone;
- prefer well-maintained crates with a compatible license.

Keep `Cargo.lock` committed; this workspace ships a single lockfile so CI and
releases are reproducible.

### Test fixtures

Fixtures are real directory trees written to a `tempfile::TempDir`, not mocks.
This is an analyzer: almost every interesting failure is a disagreement with
what is actually on disk — `.gitignore` semantics, a file that is not UTF-8, a
manifest cargo will not resolve — and none of those are observable through a
mock of the filesystem.

A fixture that needs `cargo metadata` to resolve must have no dependencies, so
the suite passes on a machine with no network. A test that genuinely needs the
network is gated and named `live_*`.

## Testing

- Module-local unit tests live in `crates/<crate>/src/<feature>/test.rs` and may
  touch private items.
- Integration tests live in `crates/<crate>/tests/` and exercise only the public
  API — they are the regression suite for the crate's contract.
- Report types pin their serialized representation in an integration test. That
  representation is the report format: anything storing or diffing a report
  depends on the field names and on the `Rule` identifiers.
- Test modules open with
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`. The
  workspace forbids those in library code and they are the right thing in a
  test; the allow is per test module so it can never leak into a `mod.rs`.
- A rule is tested at its boundary: one measurement below the threshold
  producing nothing, one at it producing a finding. A test that only uses
  obviously extreme inputs cannot see a rule that fires one unit early.
- Use descriptive, behavioral test names: `rejects_an_empty_name`, not
  `test_greet_2`.
- Cover the failure paths, not just the happy path. Every new error variant
  needs a test that produces it.
- For async behavior, standardize on one runtime (`tokio` as a dev-dependency
  for tests) rather than mixing runtimes.
- Tests must be deterministic and independent of network, wall-clock time, and
  execution order. Gate any live/network test behind a feature or an env var and
  name it `live_*` so it is easy to exclude.
- Maintain at least 90% line coverage in every source file. Add or update tests
  with every behavior change, and note any deliberately untested edge case in
  the pull request description.

Write the test first when fixing a bug: a failing test that reproduces the
report, then the fix that turns it green.

## Documentation

Write documentation for the reader who has never seen the code.

- Every public item gets a rustdoc comment. `missing_docs` is a warning that CI
  treats as an error.
- Start every `mod.rs` and `test.rs` with a concise module-level `//!`
  description.
- Each crate's `src/lib.rs` carries its crate-level overview: what the crate
  does, the primary entry points, and a short runnable example. It should also
  say what the crate deliberately does *not* hold, and why.
- Prefer concrete examples over vague description. Doc examples are compiled and
  run by `cargo test`, so they cannot drift.
- Complex modules must include a module-level `README.md` covering their design,
  public surface, and important operational constraints.
- Keep `README.md`, `docs/`, and module docs aligned with code changes in the
  same commit that changes behavior.
- Write accepted behavior and constraints in `docs/specs/` before creating a
  linked, implementation-ordered plan in `docs/plans/`. Specs define what and
  why; plans define how and in what sequence.
- Keep every Markdown file, including this one, at 500 lines or fewer. When a
  topic outgrows that, split it into focused files and link them from the
  nearest `README.md`.

## Git Workflow

- Never commit directly to `main`. Branch first, one branch per logical change.
- Do feature work in a git worktree so the main checkout stays clean.
- Commit subjects are concise and imperative: `Add retry policy to the client`.
  Keep the subject specific to the change and under ~72 characters.
- Make small, focused commits. Each commit should cover one logical change,
  build independently, and avoid mixing formatting, refactors, and behavior
  changes unless they are inseparable.
- Never commit secrets. `.env` is git-ignored; document new variables in
  `.env.example` with placeholder values.
- Never force-push a shared branch, rewrite published history, or bypass hooks
  with `--no-verify`.

## Pull Requests

Open pull requests ready for review, not as drafts, unless the work genuinely
must not merge yet. A pull request should:

- summarize what changed and why, in a few sentences;
- call out public API or behavior changes explicitly, or state "None";
- list the validation commands actually run, with their outcome;
- link the related issue;
- include updated tests, docs, and examples in the same change.

The template in `.github/PULL_REQUEST_TEMPLATE.md` encodes this checklist.
Address review feedback by fixing it, and reply on each thread describing what
changed. Do not resolve a thread whose feedback you have not addressed or
explicitly declined with a reason.

## Releases

Releases run from `.github/workflows/release.yml` via a manual
`workflow_dispatch` with a `patch` / `minor` / `major` bump; `current` resumes
an interrupted release after its version commit and tag exist. The workflow
re-runs the full validation suite, computes the next version, updates
the root `[workspace.package]` version and `Cargo.lock`, commits and tags
`vX.Y.Z`, builds the `tinyanalyzer` binary for every supported platform,
verifies each one starts, and creates a GitHub release with checksummed
archives.

Consequently:

- Do not hand-edit the `version` field in the root `[workspace.package]`; the
  release workflow owns it. Every member inherits it with
  `version.workspace = true`, so the whole workspace releases as one version.
- Follow semantic versioning. Any change to the public surface that is not
  purely additive is a breaking change and needs a major bump (pre-1.0: a minor
  bump).
- The binary must build and start on every release target — `main` should always
  be green.

## Agent Working Agreement

For automated contributors specifically:

1. **Read before writing.** Inspect the surrounding module and match its
   conventions, comment density, and idiom rather than importing a house style.
2. **Verify, do not assume.** Run the four contract commands and read their
   output before reporting a task complete. Report failures with the output;
   never claim a check passed that you did not run.
3. **Stay in scope.** Implement what was asked. Do not opportunistically
   refactor, reformat, upgrade dependencies, or "fix" unrelated code — raise it
   instead.
4. **No placeholders in delivered code.** No `todo!()`, no stubbed functions, no
   commented-out alternatives left behind. If something cannot be finished, say
   so explicitly.
5. **Do not weaken the guardrails.** Never add blanket `#[allow(...)]`, relax a
   lint, mark a test `#[ignore]`, or loosen CI to get a green run. Fix the
   cause.
6. **Secrets stay out.** Never read, echo, or commit `.env` contents, tokens, or
   credentials, and never paste them into a pull request or issue.
7. **Ask only when blocked.** Make routine judgment calls yourself; escalate
   only irreversible decisions or genuine forks with no clear default.
