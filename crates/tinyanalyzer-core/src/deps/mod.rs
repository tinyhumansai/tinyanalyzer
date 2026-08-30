//! Reading the resolved dependency graph.
//!
//! The graph comes from `cargo metadata` rather than from re-parsing manifests.
//! That is the whole design decision of this module: features, optional
//! dependencies, platform-specific edges, and version unification are decided
//! by the resolver, and a tool that re-implements any of them will disagree
//! with the build it is describing. Being slower and correct beats being
//! instant and plausible.
//!
//! What this module adds on top of cargo's answer is the arithmetic cargo does
//! not do:
//!
//! - **Exclusive weight.** How many crates would leave the build if one direct
//!   dependency were dropped. The raw transitive count of a popular crate is
//!   mostly crates something else already pulls in, so it flatters and misleads;
//!   the exclusive count is what a removal would actually buy.
//! - **Duplicates.** Crates resolved at more than one version, each one a second
//!   copy compiled and linked.
//! - **Unused candidates.** Declared dependencies that no source file names.
//!   A heuristic, and labelled as one.

mod types;

pub use types::{
    DependencyEdge, DependencyKind, DependencyReport, DuplicateVersions, PackageNode,
    UnusedDependency,
};

use crate::config::DependencyConfig;
use crate::error::{Error, Result};
use cargo_metadata::MetadataCommand;
use ignore::WalkBuilder;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

/// Which crate names each workspace member's source files mention.
///
/// Keyed by package name. Built by the caller, which is the only layer that
/// knows how to map a file path back to the package that owns it.
pub type CrateReferences = BTreeMap<String, BTreeSet<String>>;

/// Resolves and measures the dependency graph rooted at `root`.
///
/// # Errors
///
/// Returns [`Error::CargoMetadata`] if cargo cannot resolve the workspace —
/// there is no manifest, the lockfile cannot be produced, or a dependency does
/// not exist. There is no partial answer to fall back on: an unresolved graph
/// is not a smaller graph, it is an unknown one.
pub fn analyze(
    root: impl AsRef<Path>,
    config: &DependencyConfig,
    references: &CrateReferences,
) -> Result<DependencyReport> {
    let root = root.as_ref();
    let metadata = resolved_metadata(root)?;

    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| Error::CargoMetadata {
            root: root.to_path_buf(),
            message: "cargo returned no resolved graph".to_owned(),
        })?;

    let members: BTreeSet<String> = metadata
        .workspace_members
        .iter()
        .map(ToString::to_string)
        .collect();

    let mut edges = Vec::new();
    let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for node in &resolve.nodes {
        let from = node.id.to_string();

        for dep in &node.deps {
            let kinds = edge_kinds(dep, config.include_dev);
            if kinds.is_empty() {
                continue;
            }

            let to = dep.pkg.to_string();
            adjacency.entry(from.clone()).or_default().push(to.clone());

            for kind in kinds {
                edges.push(DependencyEdge {
                    from: from.clone(),
                    to: to.clone(),
                    kind,
                });
            }
        }
    }

    let depths = shortest_depths(&members, &adjacency);
    let direct = direct_dependencies(&members, &adjacency);
    let member_ids: Vec<String> = members.iter().cloned().collect();
    let mut included = reachable_from(&member_ids, &adjacency);
    included.extend(members.iter().cloned());

    let mut packages = Vec::new();
    for node in &resolve.nodes {
        let id = node.id.to_string();
        if !included.contains(&id) {
            continue;
        }
        let Some(package) = metadata.packages.iter().find(|entry| entry.id == node.id) else {
            continue;
        };

        let reachable = reachable_from(std::slice::from_ref(&id), &adjacency);
        let exclusive = exclusive_weight(&id, &members, &adjacency, &reachable);

        packages.push(PackageNode {
            is_workspace_member: members.contains(&id),
            is_root_package: package.manifest_path.as_std_path() == root.join("Cargo.toml"),
            is_direct: direct.contains(&id),
            kinds: kinds_for(&id, &edges),
            features: node.features.iter().map(ToString::to_string).collect(),
            available_features: package.features.keys().map(ToString::to_string).collect(),
            transitive_count: reachable.len(),
            exclusive_count: exclusive,
            depth: depths.get(&id).copied().unwrap_or(usize::MAX),
            name: package.name.to_string(),
            version: package.version.to_string(),
            source_bytes: package_source_bytes(
                package
                    .manifest_path
                    .parent()
                    .map_or(root, |path| path.as_std_path()),
            ),
            id,
        });
    }

    packages.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    edges.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.kind.cmp(&right.kind))
    });

    let external_packages = packages
        .iter()
        .filter(|package| !package.is_workspace_member)
        .count();
    let max_depth = packages
        .iter()
        .filter(|package| package.depth != usize::MAX)
        .map(|package| package.depth)
        .max()
        .unwrap_or(0);

    Ok(DependencyReport {
        duplicates: find_duplicates(&packages),
        unused: find_unused(&metadata, &members, config, references),
        packages,
        edges,
        external_packages,
        max_depth,
    })
}

/// Asks Cargo for the workspace graph, preserving its diagnostic on failure.
fn resolved_metadata(root: &Path) -> Result<cargo_metadata::Metadata> {
    MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .exec()
        .map_err(|source| Error::CargoMetadata {
            root: root.to_path_buf(),
            message: source.to_string(),
        })
}

/// Measures the checked-out source Cargo would compile for one package.
fn package_source_bytes(root: &Path) -> u64 {
    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .filter_entry(|entry| {
            entry.depth() == 0 || !matches!(entry.file_name().to_str(), Some("target" | ".git"))
        })
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.metadata().ok())
        .fold(0_u64, |total, metadata| {
            total.saturating_add(metadata.len())
        })
}

/// The dependency kinds one resolved edge carries.
///
/// An edge can be several kinds at once — a crate used both normally and by the
/// test suite — and cargo reports each separately. Development edges are
/// dropped entirely when the configuration excludes them, which is what makes
/// "what does the shipped binary actually cost" answerable.
fn edge_kinds(dep: &cargo_metadata::NodeDep, include_dev: bool) -> Vec<DependencyKind> {
    let mut kinds: Vec<DependencyKind> = dep
        .dep_kinds
        .iter()
        .filter_map(|info| match info.kind {
            cargo_metadata::DependencyKind::Normal => Some(DependencyKind::Normal),
            cargo_metadata::DependencyKind::Development if include_dev => {
                Some(DependencyKind::Development)
            }
            cargo_metadata::DependencyKind::Build => Some(DependencyKind::Build),
            _ => None,
        })
        .collect();

    // An edge with no recorded kinds is a normal dependency; older metadata
    // versions omit the list entirely rather than writing it out.
    if dep.dep_kinds.is_empty() {
        kinds.push(DependencyKind::Normal);
    }

    kinds.sort_unstable();
    kinds.dedup();
    kinds
}

/// Every kind by which some edge reaches `id`.
fn kinds_for(id: &str, edges: &[DependencyEdge]) -> Vec<DependencyKind> {
    let mut kinds: Vec<DependencyKind> = edges
        .iter()
        .filter(|edge| edge.to == id)
        .map(|edge| edge.kind)
        .collect();

    kinds.sort_unstable();
    kinds.dedup();
    kinds
}

/// Shortest distance from any workspace member to every reachable package.
fn shortest_depths(
    members: &BTreeSet<String>,
    adjacency: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, usize> {
    let mut depths: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    for member in members {
        depths.insert(member.clone(), 0);
        queue.push_back((member.clone(), 0));
    }

    while let Some((id, depth)) = queue.pop_front() {
        let Some(children) = adjacency.get(&id) else {
            continue;
        };

        for child in children {
            let next = depth.saturating_add(1);
            if depths.get(child).is_none_or(|current| next < *current) {
                depths.insert(child.clone(), next);
                queue.push_back((child.clone(), next));
            }
        }
    }

    depths
}

/// Packages a workspace member names directly.
fn direct_dependencies(
    members: &BTreeSet<String>,
    adjacency: &BTreeMap<String, Vec<String>>,
) -> BTreeSet<String> {
    members
        .iter()
        .filter_map(|member| adjacency.get(member))
        .flatten()
        .cloned()
        .collect()
}

/// Every package reachable from `seeds`, excluding the seeds themselves.
fn reachable_from(seeds: &[String], adjacency: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<&String> = VecDeque::new();

    for seed in seeds {
        if let Some(children) = adjacency.get(seed) {
            queue.extend(children);
        }
    }

    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(children) = adjacency.get(id) {
            queue.extend(children);
        }
    }

    for seed in seeds {
        seen.remove(seed);
    }

    seen
}

/// How many packages would leave the build if `id` were removed.
///
/// Computed as everything reachable from `id` that is *not* reachable from the
/// workspace by any other route, plus `id` itself. This is the number a reader
/// wants when deciding whether a dependency is worth its weight: the raw
/// transitive count of a widely-used crate is mostly crates that would stay
/// regardless.
fn exclusive_weight(
    id: &str,
    members: &BTreeSet<String>,
    adjacency: &BTreeMap<String, Vec<String>>,
    reachable: &BTreeSet<String>,
) -> usize {
    if members.contains(id) {
        return reachable.len();
    }

    // Walk the graph from the workspace with every edge into `id` cut.
    let mut without: BTreeMap<String, Vec<String>> = adjacency.clone();
    for children in without.values_mut() {
        children.retain(|child| child != id);
    }

    let seeds: Vec<String> = members.iter().cloned().collect();
    let survivors = reachable_from(&seeds, &without);

    reachable
        .iter()
        .filter(|package| !survivors.contains(*package))
        .count()
        .saturating_add(1)
}

/// Crates resolved at more than one version.
fn find_duplicates(packages: &[PackageNode]) -> Vec<DuplicateVersions> {
    let mut by_name: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for package in packages {
        by_name
            .entry(package.name.as_str())
            .or_default()
            .insert(package.version.as_str());
    }

    by_name
        .into_iter()
        .filter(|(_, versions)| versions.len() > 1)
        .map(|(name, versions)| DuplicateVersions {
            name: name.to_owned(),
            versions: versions.into_iter().map(ToOwned::to_owned).collect(),
        })
        .collect()
}

/// Declared dependencies that no source file in the declaring package names.
///
/// Names are compared with hyphens folded to underscores, because that is the
/// transformation cargo applies between a manifest entry and the identifier a
/// `use` statement writes.
fn find_unused(
    metadata: &cargo_metadata::Metadata,
    members: &BTreeSet<String>,
    config: &DependencyConfig,
    references: &CrateReferences,
) -> Vec<UnusedDependency> {
    let ignored: BTreeSet<String> = config
        .ignore_unused
        .iter()
        .map(|name| normalize_crate_name(name))
        .collect();

    let mut unused = Vec::new();

    for package in &metadata.packages {
        if !members.contains(&package.id.to_string()) {
            continue;
        }

        let package_name = package.name.to_string();
        let Some(named) = references.get(&package_name) else {
            // No source files were analyzed for this member, so nothing can be
            // concluded about what it uses. Silence beats a page of false
            // positives.
            continue;
        };

        for dependency in &package.dependencies {
            let kind = match dependency.kind {
                cargo_metadata::DependencyKind::Normal => DependencyKind::Normal,
                cargo_metadata::DependencyKind::Development if config.include_dev => {
                    DependencyKind::Development
                }
                cargo_metadata::DependencyKind::Build => DependencyKind::Build,
                _ => continue,
            };

            let name = dependency
                .rename
                .clone()
                .unwrap_or_else(|| dependency.name.clone());
            let normalized = normalize_crate_name(&name);

            if ignored.contains(&normalized) || named.contains(&normalized) {
                continue;
            }

            unused.push(UnusedDependency {
                package: package_name.clone(),
                dependency: name,
                kind,
            });
        }
    }

    unused.sort_by(|left, right| {
        left.package
            .cmp(&right.package)
            .then_with(|| left.dependency.cmp(&right.dependency))
    });
    unused.dedup();
    unused
}

/// Folds a manifest crate name into the identifier a `use` statement writes.
#[must_use]
pub fn normalize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

#[cfg(test)]
mod test;
