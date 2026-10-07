//! Following `mod` declarations from every crate root.
//!
//! Cargo compiles a target by starting at one root file and following its
//! `mod` declarations. A `.rs` file no declaration reaches is never compiled:
//! it costs nothing at build time and everything at review time, because it
//! reads exactly like live code. It also distorts every other measurement —
//! an orphaned copy of a live file is, to the clone detector, an exact clone.
//!
//! Resolving the tree answers two questions:
//!
//! - **Which files are orphans?** Files under a package's `src/` that no root
//!   reaches. They are reported as dead code and kept out of clone detection.
//! - **Which files are test code?** Files reached only through a
//!   `#[cfg(test)]` declaration or a test target — the
//!   `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;` sibling convention, and
//!   any other name a path glob cannot know.
//!
//! # Being conservative
//!
//! An orphan report invites a deletion, so the resolver gives up on a package
//! rather than guess:
//!
//! - a package with an `include!` whose argument is not a string literal, a
//!   reachable file that failed to parse or was too large to read, or one the
//!   walk excluded but that exists on disk, is not searched for orphans at all;
//! - a file whose module name appears inside an item-position macro anywhere
//!   the package reaches is not reported, because the macro may expand to
//!   `mod name;`;
//! - a file with no items is not reported;
//! - roots are over-approximated: every conventional target location counts,
//!   whatever `autobins` and friends say, plus every explicit manifest path.
//!
//! Files outside `src/` — `tests/`, `examples/`, `benches/` — are never
//! reported; Cargo compiles each of their top-level files on its own.

mod declarations;
mod path;
mod roots;
mod types;

pub(crate) use declarations::collect;
pub(crate) use types::{FileDeclarations, ModuleTree, OrphanFile};

use path::{join, parent, stem};
use std::collections::{BTreeMap, BTreeSet};

/// One Rust file as the resolver sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ModuleInput<'a> {
    /// The file, relative to the analysis root.
    pub(crate) path: &'a str,
    /// What the file declares, or `None` when it could not be parsed or read.
    pub(crate) declarations: Option<&'a FileDeclarations>,
}

/// One package manifest found by the walk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PackageManifest<'a> {
    /// The directory holding `Cargo.toml`, or `"."` for the analysis root.
    pub(crate) directory: &'a str,
    /// The package's name.
    pub(crate) name: &'a str,
    /// The parsed manifest.
    pub(crate) manifest: &'a toml::Value,
}

/// Resolves every package's module tree.
///
/// `exists` answers whether a root-relative path is a file on disk; it is asked
/// only about paths the walk did not produce, to tell a declaration of an
/// excluded file (which makes a package's tree incomplete) from one of a file
/// that does not exist (which is a configuration Cargo would refuse anyway).
pub(crate) fn resolve(
    packages: &[PackageManifest<'_>],
    files: &[ModuleInput<'_>],
    exists: &dyn Fn(&str) -> bool,
) -> ModuleTree {
    let index: BTreeMap<&str, Option<&FileDeclarations>> = files
        .iter()
        .map(|file| (file.path, file.declarations))
        .collect();
    let rust_files: Vec<&str> = index.keys().copied().collect();
    let directories: BTreeSet<&str> = packages.iter().map(|package| package.directory).collect();

    let mut reached_anyhow = BTreeSet::new();
    let mut reached_in_production = BTreeSet::new();
    let mut orphans = Vec::new();

    for package in packages {
        let roots = roots::roots(package.directory, package.manifest, &rust_files);

        let mut everything = Walk::new(&index, exists);
        everything.run(roots.iter().map(|root| root.path.as_str()), true);
        let mut production = Walk::new(&index, exists);
        production.run(
            roots
                .iter()
                .filter(|root| !root.is_test)
                .map(|root| root.path.as_str()),
            false,
        );
        reached_in_production.extend(production.reached);

        if !everything.searchable() {
            reached_anyhow.extend(everything.reached);
            continue;
        }

        let Some(source) = join(package.directory, "src") else {
            reached_anyhow.extend(everything.reached);
            continue;
        };
        for file in files {
            let Some(declarations) = file.declarations else {
                continue;
            };
            let is_candidate = file
                .path
                .strip_prefix(source.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
                && owner(file.path, &directories) == Some(package.directory)
                && !everything.reached.contains(file.path)
                && declarations.items > 0
                && !everything
                    .macro_identifiers
                    .contains(module_name(file.path));
            if is_candidate {
                orphans.push(OrphanFile {
                    path: file.path.to_owned(),
                    crate_name: package.name.to_owned(),
                });
            }
        }
        reached_anyhow.extend(everything.reached);
    }

    orphans.sort();
    orphans.dedup();

    ModuleTree {
        orphans,
        test_only: reached_anyhow
            .difference(&reached_in_production)
            .cloned()
            .collect(),
    }
}

/// The name a `mod` declaration would use for `path`.
fn module_name(path: &str) -> &str {
    if stem(path) == "mod" {
        stem(parent(path))
    } else {
        stem(path)
    }
}

/// The package directory owning `path`: the nearest one at or above it.
fn owner<'a>(path: &str, directories: &BTreeSet<&'a str>) -> Option<&'a str> {
    let mut directory = parent(path);
    loop {
        if let Some(found) = directories.get(directory) {
            return Some(found);
        }
        if directory == "." {
            return None;
        }
        directory = parent(directory);
    }
}

/// One traversal of the declaration graph from a set of roots.
struct Walk<'a, 'b> {
    index: &'b BTreeMap<&'a str, Option<&'a FileDeclarations>>,
    exists: &'b dyn Fn(&str) -> bool,
    /// Every file reached.
    reached: BTreeSet<String>,
    /// Whether some reachable file's declarations are unknown.
    incomplete: bool,
    /// Whether some reachable file includes a file by a non-literal path.
    unresolved_include: bool,
    /// Identifiers inside item-position macros of reachable files.
    macro_identifiers: BTreeSet<String>,
}

impl<'a, 'b> Walk<'a, 'b> {
    fn new(
        index: &'b BTreeMap<&'a str, Option<&'a FileDeclarations>>,
        exists: &'b dyn Fn(&str) -> bool,
    ) -> Self {
        Self {
            index,
            exists,
            reached: BTreeSet::new(),
            incomplete: false,
            unresolved_include: false,
            macro_identifiers: BTreeSet::new(),
        }
    }

    /// Whether the traversal saw enough of the package to call a file an orphan.
    fn searchable(&self) -> bool {
        !self.reached.is_empty() && !self.incomplete && !self.unresolved_include
    }

    /// Whether a root-relative path names a file, walked or not.
    fn is_file(&self, path: &str) -> bool {
        self.index.contains_key(path) || (self.exists)(path)
    }

    /// Follows declarations from `roots`, skipping test-only ones unless
    /// `follow_test`.
    fn run<'r>(&mut self, roots: impl Iterator<Item = &'r str>, follow_test: bool) {
        // A node is a file plus the directories its declarations resolve
        // against: one file can be reached as two different modules.
        let mut pending: Vec<(String, Vec<String>)> = roots
            .filter(|root| self.is_file(root))
            .map(|root| (root.to_owned(), vec![parent(root).to_owned()]))
            .collect();
        let mut visited = BTreeSet::new();

        while let Some((file, bases)) = pending.pop() {
            if !visited.insert((file.clone(), bases.clone())) {
                continue;
            }
            let Some(Some(declarations)) = self.index.get(file.as_str()).copied() else {
                // Excluded from the walk, unparsable, or unreadable: its own
                // declarations are unknown, so the package is too.
                self.incomplete = true;
                continue;
            };

            self.reached.insert(file.clone());
            self.unresolved_include |= declarations.unresolved_include;
            self.macro_identifiers
                .extend(declarations.macro_identifiers.iter().cloned());

            let directory = parent(&file);
            for declaration in &declarations.modules {
                if declaration.is_test && !follow_test {
                    continue;
                }
                for target in targets(directory, &bases, declaration) {
                    if self.is_file(&target.0) {
                        pending.push(target);
                    }
                }
            }

            for include in &declarations.includes {
                let Some(target) = join(directory, &include.path) else {
                    continue;
                };
                if include.is_source {
                    // An included file's contents belong to the including
                    // module, so its declarations resolve where ours do.
                    if self.is_file(&target) {
                        pending.push((target, bases.clone()));
                    }
                } else if self.index.contains_key(target.as_str()) {
                    self.reached.insert(target);
                }
            }
        }
    }
}

/// Every file a declaration may load, each with the directories its own
/// declarations resolve against.
fn targets(
    directory: &str,
    bases: &[String],
    declaration: &types::ModuleDeclaration,
) -> Vec<(String, Vec<String>)> {
    let inline = declaration.inline_path.join("/");
    let module_bases: Vec<String> = bases
        .iter()
        .filter_map(|base| join(base, &inline))
        .collect();
    let mut found = Vec::new();

    // `#[path]` outside any inline module is relative to the declaring file's
    // directory; inside one, to the inline module's directory.
    let path_bases = if declaration.inline_path.is_empty() {
        vec![directory.to_owned()]
    } else {
        module_bases.clone()
    };
    for value in &declaration.paths {
        for base in &path_bases {
            if let Some(target) = join(base, value) {
                // A file loaded through `#[path]` resolves its own children
                // like a `mod.rs`; both readings are followed, since a second
                // candidate can only hide an orphan, never invent one.
                let own = parent(&target).to_owned();
                let nested = join(&own, stem(&target)).unwrap_or_else(|| own.clone());
                found.push((target, vec![own, nested]));
            }
        }
    }

    if declaration.paths.is_empty() || !declaration.path_is_unconditional {
        for base in &module_bases {
            let Some(child) = join(base, &declaration.name) else {
                continue;
            };
            for candidate in [format!("{}.rs", declaration.name), format!("{}/mod.rs", declaration.name)] {
                if let Some(target) = join(base, &candidate) {
                    found.push((target, vec![child.clone()]));
                }
            }
        }
    }

    found
}

#[cfg(test)]
mod test;
