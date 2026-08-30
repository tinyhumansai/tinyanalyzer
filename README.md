# tinyanalyzer

Point it at a Rust repository. It reads every file, parses every `.rs`, resolves
the dependency graph, works out what nothing references, and opens a terminal
dashboard over the result — ranked so the first thing you see is the thing most
worth fixing.

```sh
tinyanalyzer                 # analyze the current directory, open the dashboard
tinyanalyzer ../some/repo    # analyze somewhere else
tinyanalyzer --output summary   # print a ranked text report instead
tinyanalyzer --output json      # print the whole report as JSON
```

One binary. No runtime, no browser, no bundled web assets, nothing fetched at
startup: the dashboard *is* the program.

## Installation

On Linux or macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/tinyhumansai/tinyanalyzer/main/install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/tinyhumansai/tinyanalyzer/main/install.ps1 | iex
```

The installers detect the platform, download the matching archive from the
latest GitHub release, verify it against the release's `SHA256SUMS`, and install
the binary for the current user. Set `TINYANALYZER_VERSION` to install a specific
release (for example `v0.2.1`) or `TINYANALYZER_INSTALL_DIR` to choose the
destination. Linux and macOS default to `~/.local/bin`; Windows defaults to
`%LOCALAPPDATA%\Programs\tinyanalyzer\bin` and adds it to the user `PATH`.

If you prefer to inspect an installer before running it, download the script,
read it, and execute the local copy instead of piping it into the shell.

## What it tells you

**Where the weight is.** Every file with its lines of code, comments, items,
functions, and a single weight score that ranks a dense two-hundred-line state
machine above a thousand lines of constants — because "what should I look at
first" is not the same question as "what is longest".

**What each dependency actually costs.** Not the transitive count, which is
mostly crates something else pulls in anyway, but the *exclusive* count: how
many crates would leave the build entirely if you dropped this one. Plus every
crate resolved at two versions, and every declared dependency no source file
names.

**What nothing references.** A workspace-wide identifier census finds items with
no caller anywhere. It sees through macro invocations, which an AST walk cannot,
and it tells you how sure it is: private items come back at high confidence,
public ones at medium, because a library's callers may not be in this repository.

**What is costing you at runtime.** Allocations inside loop bodies, loops nested
inside loops, `dyn` dispatch sites, monomorphized generics, `unwrap` and `expect`
outside test code.

**What to do about all of it.** Every finding carries the measurement *and* a
specific remedy. "This file is large" is not actionable; "629 lines across 23
items, split along the seams already in it" is.

## The dashboard

```
 tinyanalyzer   58 files · 8339 loc · 508 functions · 216 crates · 382.0 KiB
 1·Overview  2·Files  3·Directories  4·Dependencies  5·Dead code  6·Findings
 ▒▒▒▒▒▒  ▒▒  ▒▒▄   ▒▒  ▒▒   ▒▒  ▄▒▒▒▒▄  ▒▒▄   ▒▒  ▄▒▒▒▒▄  ▒▒     ▒▒   ▒▒  ▒▒▀▀▒▄  ▄▒▒▒▒▒  ▒▒▀▒▒▄
   ▓▓    ▓▓  ▓▓▀▓▄▓▓  ▓▓   ▓▓  ▓▓   ▓▓  ▓▓▀▓▄▓▓  ▓▓   ▓▓  ▓▓     ▓▓   ▓▓  ▀▀   ▓▓  ▓▓   ▓▓  ▓▓   ▓▓
 ┘┘▀▀┘┘  ▀▀  ▀▀┘┘▀▀▀  ▀▀▀  ▀▀  ▀▀  ┘▀▀  ▀▀┘┘▀▀▀  ▀▀  ┘▀▀  ▀▀┘┘┘  ▀▀▀  ▀▀     ▀▀▀  ▀▀┘┘┘   ▀▀▀▀▀
   ▓▓    ▓▓  ▓▓    ▓▓   ▀▀▀▓▓  ▓▓▀▀▓▓  ▓▓    ▓▓  ▓▓▀▀▓▓  ▓▓      ▀▀▀▓▓  ▄▓▀▀▀   ▓▓▀▀    ▓▓   ▓▓
   ▒▒    ▒▒  ▒▒    ▒▒ ▄▄   ▒▒  ▒▒   ▒▒  ▒▒    ▒▒  ▒▒   ▒▒  ▒▒    ▄▄   ▒▒  ▒▒  ▄▄  ▒▒   ▒▒  ▒▒   ▒▒
   ░░    ░░  ░░    ░░  ░░▄▄░▀  ░░   ░░  ░░    ░░  ░░   ░░  ░░░░░  ░░▄▄░▀  ▀░░▄░░  ▀░░░░░  ░░   ░░
┌ Totals ───────────────────────┐┌ Languages by lines of code ─────────────┐
│ files                  58     ││ ██████████                              │
│ lines of code        8339     ││ ██████████                              │
│ functions             508     ││ ██████████       ▃▃▃▃▃▃                 │
│ allocations in loops   56     ││ Rust           Markdown         TOML    │
└───────────────────────────────┘└─────────────────────────────────────────┘
┌ Top findings (51) ──────────────────────────────────────────────────────┐
│ high    crates/…/dashboard/render.rs is 629 lines of code               │
│ high    crates/…/deps/mod.rs allocates 20 times inside loops            │
│ high    Dashboard::apply has 19 paths through it                        │
└─────────────────────────────────────────────────────────────────────────┘
 q quit · tab/1-6 view · ↑↓ move · t tests · / filter · tests shown
```

On wide terminals the colored wordmark occupies only the left column above
Totals; Languages stays pinned to the top-right. The wordmark disappears when
that column is too narrow or short, preserving space for both data panels.

| Key | What it does |
|---|---|
| `q`, `Esc`, `Ctrl-C` | Leave; in the focused dependency sidebar, `Esc` backs out first |
| `Tab`, `Shift-Tab`, `1`–`6` | Change view |
| `↑` `↓`, `j` `k` | Move the cursor |
| `PgUp` `PgDn`, `u` `d` | Move a screenful |
| `Home` `End`, `g` `G` | First and last row |
| `t` | Show or hide test code, everywhere |
| `/` | Filter rows with a case-insensitive regex; `Enter` keeps it, `Esc` clears it |
| `s` | Cycle the current pane's sort order |
| `i` | Toggle whether discovery follows `.gitignore` and rebuild the report |
| `d`, `r` in Dependencies | Toggle mock removal for the selected dependency; restore all removals |
| `[`, `]`, `f`, `w` in Dependencies | Select/toggle a Cargo feature; switch dependency/root target |
| `Enter`, `→`, `l` / `Esc`, `Backspace`, `←`, `h` in Dependencies | Focus/drill into the right dependency sidebar / return one level |
| `Enter`, `→`, `l` / `Backspace`, `←`, `h` | Enter / leave a directory |
| `o` in Directories | Toggle between directories and directories + files |
| Mouse | Click tabs and rows, wheel to move, right-click to leave a directory |

The `t` filter is the one worth knowing about: it removes test code from every
row *and* every total at once, which is usually the honest answer to "how big is
this project". For Rust, that includes `#[test]` functions and `#[cfg(test)]`
modules inside otherwise-production files, not only files under `tests/`.

Filters are case-insensitive regular expressions, so `^crates/.*/src/.*\.rs$`
can isolate Rust source below workspace crates. While a pattern is incomplete,
the dashboard treats it literally and labels it `invalid regex` instead of
blanking the pane unexpectedly.

In Dependencies, `d` runs a reversible removal simulation. The selected direct
dependency stays in the list marked as mock-removed, workspace reachability is
recomputed, and the graph lists every crate that would become unreachable. The
headline shows direct and total dependency counts plus the exact number of
external crates and checked-out dependency source bytes the simulated build
graph retains. This is source size, not a target-specific prediction of linked
binary size. Press `d` again on a marked dependency to restore it, use `d` on
other rows to model several removals, or press `r` to restore the whole graph.
Each direct dependency row shows its own source size, and `s` can sort the pane
by that column after cycling through exclusive, name, and reachable-crate order.
Every child in the selected dependency tree also shows its checked-out source
size and immediate child-dependency count, so expensive branches are visible
without changing the selection. Press `Enter` to focus that sidebar, use the
arrow keys to select a child, and press `Enter` again to drill down; `Esc`
returns one level and eventually restores focus to the direct-dependency list.
The active `s` ordering applies to both sidebars.
The `exclusive` and `reaches` counts are recalculated after every toggle, so
shared transitive crates move to whichever remaining direct dependency now owns
their cost; removed rows show zero until restored.

The dependency detail also exposes Cargo features. `[` and `]` select a feature,
`f` toggles it in the simulation, and `w` switches between the selected direct
dependency and the root workspace package. Feature state is modeled without
editing manifests; because Cargo must resolve optional dependencies, the pane
labels graph effects as pending a fresh analysis rather than inventing them.

## Configuration

`tinyanalyzer.toml` in the repository root, and every part of it is optional —
an unconfigured repository gets a useful report, and the file only ever records
deviations from that. `tiny-analyzer.toml` is accepted too.

```toml
[project]
name = "my-project"
description = "what it is"

[scan]
exclude = ["generated/**"]        # added to the defaults, not replacing them
test_patterns = ["**/fixtures/**"]
respect_gitignore = true
max_file_bytes = 2_000_000

[thresholds]
large_file_lines = 400
huge_file_lines = 800
long_function_lines = 60
high_complexity = 15
heavy_dependency_crates = 20
min_comment_ratio = 0.05

[dead_code]
ignore = ["main", "some_macro_target"]
tests_count_as_uses = false       # an item only its tests call is dead weight

[dependencies]
ignore_unused = ["thiserror"]     # reached only through a derive macro

[ui]
start_view = "findings"
hide_tests = true

# Things to note: a path and a sentence saying why it looks the way it does.
# A file the team already knows is huge, with a note saying so, reads very
# differently from one nobody has looked at.
[[notes]]
path = "crates/parser/src/**"
note = "hand-written parser; the long functions are deliberate"
level = "info"                    # info | warning | critical
```

This repository's own [`tinyanalyzer.toml`](tinyanalyzer.toml) is the worked
example.

## Command line

| Flag | What it does |
|---|---|
| `<PATH>` | Repository to analyze. Defaults to `.` |
| `-o, --output <dashboard\|summary\|json>` | What to do with the report |
| `--write <FILE>` | Write the output to a file instead of stdout |
| `-c, --config <FILE>` | Use this configuration instead of looking for one |
| `--view <VIEW>` | Open the dashboard on `overview`, `files`, `dependencies`, `dead-code`, or `findings` |
| `--hide-tests` | Exclude test code from every total on startup |
| `--no-deps` | Skip the dependency graph — pure filesystem work |
| `--no-dead-code` | Skip dead-code detection |
| `--hidden` | Include dotfiles and dot-directories |
| `--no-ignore` | Analyze files `.gitignore` would exclude |

Flags override the configuration file rather than replacing it: `--no-deps`
against a repository with configured thresholds keeps those thresholds.

## As a library

The analysis engine is its own crate with no terminal, no renderer, and no
argument parser in it, so it can be embedded in a CI check, an editor plugin, or
anything that wants the numbers without the interface.

```rust
use tinyanalyzer_core::{Report, analyze};

let report: Report = analyze(".")?;

for finding in report.findings.iter().take(5) {
    println!("[{}] {}", finding.severity.label(), finding.title);
    println!("    {}", finding.suggestion);
}
# Ok::<(), tinyanalyzer_core::Error>(())
```

The report is fully serializable, and `schema_version` moves whenever a field is
removed or changes meaning — so two reports from two commits can be diffed to
see what a refactor actually did.

## What it approximates, and how

Every measurement here is either exact or documented as an approximation. The
three that are worth knowing about before you act on them:

- **Cyclomatic complexity** counts branches, not paths: `if`, `match` arms,
  loops, `&&`, `||`, `?`. A `match` whose arms all name constants is a lookup
  table, not a decision, and is not counted — otherwise every exhaustive `match`
  over an enum would rank as the worst function in the repository.
- **Dead code** is an identifier census, not name resolution. Two unrelated items
  with the same name vouch for each other, so it under-reports rather than
  over-reports — the right direction for a list somebody is going to act on.
  Read `crates/tinyanalyzer-core/src/dead_code/mod.rs` before deleting anything.
- **Unused dependencies** are dependencies no source file *names*. A crate
  reached through a macro expansion, a build script, or a linker side effect has
  no `use` naming it. Remove and build; if the build fails, add it to
  `ignore_unused`.

Files that do not parse are reported as findings rather than silently dropped,
because a file missing from the Rust-level measurements reads as a clean one.

## Building

```sh
cargo build --release
./target/release/tinyanalyzer
```

Requires Rust 1.88 or newer. The four commands CI runs, which a local run should
reproduce exactly:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
```

## Layout

```text
crates/
├── tinyanalyzer-core/   # the analysis engine: no terminal, no CLI, no server
│   ├── config/          # tinyanalyzer.toml
│   ├── walk/            # which files are in scope
│   ├── loc/             # code, comment, and blank lines
│   ├── rust_source/     # syn-based items, complexity, cost signals
│   ├── deps/            # the resolved dependency graph and its real cost
│   ├── dead_code/       # the workspace-wide identifier census
│   ├── findings/        # the rules that turn measurements into advice
│   └── report/          # all of it, joined and ranked
└── tinyanalyzer/        # the binary: command line, text output, dashboard
    ├── cli/
    ├── summary/
    └── dashboard/       # state machine, renderer, event loop
```

CI asserts that the engine never grows a dependency on a terminal UI, an
argument parser, a server, or an async runtime.

## Contributing

See [`AGENTS.md`](AGENTS.md) for how work is done in this repository —
`CLAUDE.md` is a symlink to the same file — and
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the pull request process.

## License

GPL-3.0-only. See [`LICENSE`](LICENSE).
