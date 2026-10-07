//! Finding every file Cargo compiles as the root of a target.
//!
//! The conventional locations (`src/lib.rs`, `src/main.rs`, `src/bin/`,
//! `build.rs`, `examples/`, `tests/`, `benches/`) are always roots, whatever
//! `autobins` and friends say, and every explicit `path` in the manifest is
//! added on top. Over-counting roots can only hide an orphan; under-counting
//! one would report a whole target as dead, which is the error that matters.

use super::path::join;

/// One root file and whether its target is test code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Root {
    /// The root file, relative to the analysis root.
    pub(crate) path: String,
    /// Whether the target is a test or a benchmark.
    pub(crate) is_test: bool,
}

/// Every root of the package whose manifest sits in `directory`.
///
/// `rust_files` is every Rust file the walk found, used to enumerate the
/// auto-discovered target directories. `manifest` is the parsed `Cargo.toml`.
pub(crate) fn roots(directory: &str, manifest: &toml::Value, rust_files: &[&str]) -> Vec<Root> {
    let mut roots = Vec::new();
    let mut add = |relative: &str, is_test: bool| {
        if let Some(path) = join(directory, relative) {
            roots.push(Root { path, is_test });
        }
    };

    for fixed in ["src/lib.rs", "src/main.rs", "build.rs"] {
        add(fixed, false);
    }
    if let Some(build) = manifest
        .get("package")
        .and_then(|package| package.get("build"))
        .and_then(toml::Value::as_str)
    {
        add(build, false);
    }
    if let Some(lib) = manifest
        .get("lib")
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
    {
        add(lib, false);
    }
    for (table, is_test) in [
        ("bin", false),
        ("example", false),
        ("test", true),
        ("bench", true),
    ] {
        let targets = manifest.get(table).and_then(toml::Value::as_array);
        for target in targets.into_iter().flatten() {
            if let Some(path) = target.get("path").and_then(toml::Value::as_str) {
                add(path, is_test);
            }
        }
    }

    for (target_directory, is_test) in [
        ("src/bin", false),
        ("examples", false),
        ("tests", true),
        ("benches", true),
    ] {
        let Some(prefix) = join(directory, target_directory) else {
            continue;
        };
        for file in rust_files {
            if is_auto_target(file, &prefix) {
                roots.push(Root {
                    path: (*file).to_owned(),
                    is_test,
                });
            }
        }
    }

    roots.sort_by(|left, right| left.path.cmp(&right.path));
    roots.dedup_by(|later, earlier| {
        // A file that is both a production and a test root is production.
        if later.path == earlier.path {
            earlier.is_test = earlier.is_test && later.is_test;
            true
        } else {
            false
        }
    });
    roots
}

/// Whether `file` is `<prefix>/<name>.rs` or `<prefix>/<name>/main.rs`, the
/// two shapes Cargo auto-discovers a target in.
fn is_auto_target(file: &str, prefix: &str) -> bool {
    let Some(rest) = file
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return false;
    };
    match rest.split_once('/') {
        None => rest.ends_with(".rs"),
        Some((_, inner)) => inner == "main.rs",
    }
}
