# 2. Tree-sitter for structural analysis

- **Status:** Accepted
- **Date:** 2026-10-06

## Context

Clone detection needs to compare the *structure* of code across a whole
repository: normalize names away, hash every subtree, line two fragments up
node by node, and measure an edit distance between them. The engine already
parses Rust with `syn`, but `syn` is a typed AST with one Rust type per node
kind. Every one of those operations would have to be written once per node
type, and none of it would carry over to a second language.

Tree-sitter produces a concrete syntax tree whose nodes are all the same type,
named by a string kind, with named fields. A normalizer, a hasher, or an
anti-unifier written against it is written once, and adding a language is
adding a grammar. It is also error-tolerant: a file with a syntax error still
produces a tree with the damage confined to `ERROR` nodes, where `syn` refuses
the whole file.

## Decision

Clone detection (`crates/tinyanalyzer-core/src/clones/`) parses with
`tree-sitter` and `tree-sitter-rust`. `syn` remains the parser behind every
existing per-item measurement — item counts, complexity, nesting, performance
signals, the identifier census — and none of those change.

Each file is parsed twice, once by each parser. That costs about a second on a
115,000-line repository and keeps the two analyses independent: neither can
regress the other, and either can be replaced alone.

## Consequences

- The grammar is C, compiled by `cc` at build time. Every release target needs
  a C toolchain; all five in `.github/workflows/release.yml` (Linux x86-64 and
  aarch64, macOS aarch64 and x86-64, Windows MSVC) have one by default.
- The engine still pulls in no terminal UI, argument parser, server, or async
  runtime; the CI check that asserts it is unchanged and passes.
- Both crates are MIT-licensed, which `deny.toml` already allows.
- Another language is a new grammar crate, behind a Cargo feature, plus its
  own fragment-kind table — not a new detector.
- Inside a macro invocation tree-sitter sees a token tree, not syntax, so
  clone detection is blind to structure there. The analysis contract says so.
