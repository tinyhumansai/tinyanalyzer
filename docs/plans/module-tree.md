# Plan: the module tree — orphaned files and test-only files

**Specification:** [`../specs/module-tree.md`](../specs/module-tree.md)
**Status:** complete

## Goal

Implement the specification in test-first steps, each of which builds and
passes the four contract commands on its own.

## Tasks

1. **Default glob** — `config/types.rs`. Test in `walk/test.rs` that
   `src/x_tests.rs` and `src/x_test.rs` are test paths; add `**/*_tests.rs`.
2. **Path arithmetic** — `module_tree/path.rs`. Tests: `.` and `..` folding,
   absolute paths and paths above the root refused, parent and stem.
3. **Declarations** — `module_tree/declarations.rs`, run on the syntax tree
   `rust_source::analyze_with_declarations` already built. Tests: a
   `#[cfg(test)] #[path]` declaration, inline nesting with and without
   `#[path]`, file-level `#![cfg(test)]`, `cfg_attr` paths, `mod` inside an
   item macro, literal and computed `include!`.
4. **Roots** — `module_tree/roots.rs`. Tests: every conventional location and
   every manifest `path`; a file that is both a test and a production root is
   production.
5. **Resolution** — `module_tree/mod.rs`. Tests: orphan found; `mod.rs` and
   named-file layouts; `#[path]` and inline nesting; conditional paths;
   test-only through `#[cfg(test)]`; a test target reaching `src/` through
   `#[path]`; includes; every refusal in the specification's invariants;
   nested packages.
6. **Report, dead code, findings** — `report/`, `dead_code/`, `findings/`,
   `DefinitionKind::File`, `Rule::OrphanFile`. Tests: test-only files are
   test code and leave the census; an orphan is one `file` candidate, an
   `orphan_file` finding, and no clone; dead code off drops orphans; the
   `dead_code` finding no longer counts files. Pin the serialized `file` kind
   and `orphan_file` identifier in `tests/public_api.rs`.
7. **Real repositories.** Run the binary against `tinyagents` and confirm
   `ops/team.rs` and `ops/rows.rs` are orphans; run it against a repository
   using `*_tests.rs` siblings and compare test classification before and
   after.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
.github/scripts/check-file-coverage.sh 90
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

## Completion checklist

- [x] Tasks 1–7
- [x] `docs/specs/analysis-contract.md` describes the orphan and test-only
  approximations
- [x] READMEs list the new module
