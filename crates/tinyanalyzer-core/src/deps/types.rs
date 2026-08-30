//! The shape of a resolved dependency graph.

use serde::{Deserialize, Serialize};

/// Everything the analyzer learned about a workspace's dependencies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyReport {
    /// Every package in the resolved graph, workspace members included.
    pub packages: Vec<PackageNode>,
    /// Every resolved edge, as `(from, to)` package identifiers.
    pub edges: Vec<DependencyEdge>,
    /// Crates that appear at more than one version.
    pub duplicates: Vec<DuplicateVersions>,
    /// Declared dependencies no source file appears to name.
    pub unused: Vec<UnusedDependency>,
    /// Packages in the graph, excluding workspace members.
    pub external_packages: usize,
    /// The longest shortest-path from a workspace member to any package.
    pub max_depth: usize,
}

impl DependencyReport {
    /// The packages a workspace member depends on directly, heaviest first.
    ///
    /// "Heaviest" means [`PackageNode::exclusive_count`]: the number of crates
    /// that would leave the build entirely if this one were dropped. That is
    /// the number worth acting on, and it is usually far smaller than the raw
    /// transitive count, because most of what a popular crate pulls in is
    /// already there for other reasons.
    #[must_use]
    pub fn heaviest_direct(&self) -> Vec<&PackageNode> {
        let mut direct: Vec<&PackageNode> = self
            .packages
            .iter()
            .filter(|package| package.is_direct && !package.is_workspace_member)
            .collect();

        direct.sort_by(|left, right| {
            right
                .exclusive_count
                .cmp(&left.exclusive_count)
                .then_with(|| right.transitive_count.cmp(&left.transitive_count))
                .then_with(|| left.name.cmp(&right.name))
        });

        direct
    }

    /// Looks a package up by its resolved identifier.
    #[must_use]
    pub fn package(&self, id: &str) -> Option<&PackageNode> {
        self.packages.iter().find(|package| package.id == id)
    }
}

/// One package in the resolved graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageNode {
    /// Cargo's opaque package identifier, unique across versions.
    pub id: String,
    /// The crate name.
    pub name: String,
    /// The resolved version.
    pub version: String,
    /// Whether this package is a member of the workspace under analysis.
    pub is_workspace_member: bool,
    /// Whether this package is declared by the analyzed root `Cargo.toml`.
    #[serde(default)]
    pub is_root_package: bool,
    /// Whether a workspace member names this package in its own manifest.
    pub is_direct: bool,
    /// How the graph reaches it.
    pub kinds: Vec<DependencyKind>,
    /// Features cargo resolved for it.
    pub features: Vec<String>,
    /// Every feature declared by the package, including inactive ones.
    #[serde(default)]
    pub available_features: Vec<String>,
    /// Packages reachable from this one, excluding itself.
    pub transitive_count: usize,
    /// Packages reachable *only* through this one.
    ///
    /// The count of crates that would leave the build if this dependency were
    /// removed. Unlike [`Self::transitive_count`] it does not double-count the
    /// crates everything already depends on, so it is the honest answer to
    /// "what is this costing me".
    pub exclusive_count: usize,
    /// Shortest distance from a workspace member. Members themselves are zero.
    pub depth: usize,
}

/// One resolved dependency edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// Package identifier the edge leaves.
    pub from: String,
    /// Package identifier the edge arrives at.
    pub to: String,
    /// Whether the edge is a normal, development, or build dependency.
    pub kind: DependencyKind,
}

/// How a dependency is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// A dependency of the library or binary itself.
    Normal,
    /// A dependency of tests, examples, and benchmarks only.
    Development,
    /// A dependency of a build script only.
    Build,
}

impl DependencyKind {
    /// The word used for this kind on the dashboard.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Development => "dev",
            Self::Build => "build",
        }
    }
}

/// A crate resolved at more than one version.
///
/// Every extra version is a second copy compiled, linked, and shipped, and two
/// versions of the same type do not interoperate — which is why a duplicate is
/// worth reporting even when the build is perfectly happy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateVersions {
    /// The crate name.
    pub name: String,
    /// Every resolved version, sorted.
    pub versions: Vec<String>,
}

/// A declared dependency that no source file appears to name.
///
/// This is a heuristic and says so: a crate used only through a macro
/// expansion, a build script, or a linker side effect has no `use` naming it.
/// Confirm before removing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnusedDependency {
    /// The workspace member that declares it.
    pub package: String,
    /// The dependency's name as written in the manifest.
    pub dependency: String,
    /// How it is declared.
    pub kind: DependencyKind,
}
