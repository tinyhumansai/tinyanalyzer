//! Unit tests for dependency analysis.
//!
//! The graph arithmetic — depths, reachability, exclusive weight, duplicates —
//! is tested directly against hand-built graphs, where the right answer is
//! obvious by inspection. The `cargo metadata` call itself is exercised in the
//! crate's integration tests against a real fixture workspace, because there is
//! nothing to learn from asserting against a mock of cargo's output.

use super::{
    DependencyKind, DependencyReport, DuplicateVersions, PackageNode, direct_dependencies,
    exclusive_weight, find_duplicates, normalize_crate_name, reachable_from, shortest_depths,
};
use std::collections::{BTreeMap, BTreeSet};

fn graph(edges: &[(&str, &str)]) -> BTreeMap<String, Vec<String>> {
    let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (from, to) in edges {
        adjacency
            .entry((*from).to_owned())
            .or_default()
            .push((*to).to_owned());
    }
    adjacency
}

fn members(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

fn package(name: &str, version: &str, direct: bool, exclusive: usize) -> PackageNode {
    PackageNode {
        id: format!("{name}@{version}"),
        name: name.to_owned(),
        version: version.to_owned(),
        is_workspace_member: false,
        is_direct: direct,
        kinds: vec![DependencyKind::Normal],
        features: Vec::new(),
        transitive_count: exclusive,
        exclusive_count: exclusive,
        depth: 1,
    }
}

#[test]
fn hyphens_fold_to_underscores() {
    assert_eq!(normalize_crate_name("tinyanalyzer-core"), "tinyanalyzer_core");
    assert_eq!(normalize_crate_name("serde"), "serde");
}

#[test]
fn reachability_excludes_the_seed_itself() {
    let adjacency = graph(&[("a", "b"), ("b", "c")]);

    let reachable = reachable_from(&["a".to_owned()], &adjacency);

    assert_eq!(reachable, members(&["b", "c"]));
}

#[test]
fn reachability_terminates_on_a_cycle() {
    let adjacency = graph(&[("a", "b"), ("b", "a")]);

    let reachable = reachable_from(&["a".to_owned()], &adjacency);

    assert_eq!(reachable, members(&["b"]));
}

#[test]
fn a_leaf_reaches_nothing() {
    let adjacency = graph(&[("a", "b")]);

    assert!(reachable_from(&["b".to_owned()], &adjacency).is_empty());
}

#[test]
fn depths_are_the_shortest_path_from_a_member() {
    let adjacency = graph(&[("root", "a"), ("a", "b"), ("b", "c"), ("root", "c")]);

    let depths = shortest_depths(&members(&["root"]), &adjacency);

    assert_eq!(depths.get("root"), Some(&0));
    assert_eq!(depths.get("a"), Some(&1));
    assert_eq!(depths.get("b"), Some(&2));
    assert_eq!(
        depths.get("c"),
        Some(&1),
        "the direct edge is shorter than the path through b"
    );
}

#[test]
fn only_packages_a_member_names_are_direct() {
    let adjacency = graph(&[("root", "a"), ("a", "b")]);

    let direct = direct_dependencies(&members(&["root"]), &adjacency);

    assert_eq!(direct, members(&["a"]));
}

#[test]
fn exclusive_weight_counts_only_what_would_actually_leave() {
    // `shared` is reachable both through `a` and directly from the workspace,
    // so dropping `a` would not remove it.
    let adjacency = graph(&[
        ("root", "a"),
        ("root", "shared"),
        ("a", "shared"),
        ("a", "private"),
    ]);
    let workspace = members(&["root"]);
    let reachable = reachable_from(&["a".to_owned()], &adjacency);

    let weight = exclusive_weight("a", &workspace, &adjacency, &reachable);

    assert_eq!(reachable.len(), 2, "a reaches shared and private");
    assert_eq!(weight, 2, "a itself plus private; shared survives");
}

#[test]
fn a_dependency_that_shares_nothing_costs_its_whole_subtree() {
    let adjacency = graph(&[("root", "a"), ("a", "b"), ("b", "c")]);
    let workspace = members(&["root"]);
    let reachable = reachable_from(&["a".to_owned()], &adjacency);

    assert_eq!(
        exclusive_weight("a", &workspace, &adjacency, &reachable),
        3,
        "a, b, and c all leave together"
    );
}

#[test]
fn a_leaf_dependency_costs_exactly_itself() {
    let adjacency = graph(&[("root", "a")]);
    let workspace = members(&["root"]);
    let reachable = reachable_from(&["a".to_owned()], &adjacency);

    assert_eq!(exclusive_weight("a", &workspace, &adjacency, &reachable), 1);
}

#[test]
fn a_workspace_member_is_weighed_by_everything_it_reaches() {
    let adjacency = graph(&[("root", "a"), ("a", "b")]);
    let workspace = members(&["root"]);
    let reachable = reachable_from(&["root".to_owned()], &adjacency);

    assert_eq!(
        exclusive_weight("root", &workspace, &adjacency, &reachable),
        2
    );
}

#[test]
fn a_single_version_is_not_a_duplicate() {
    let packages = vec![package("serde", "1.0.0", true, 1)];

    assert!(find_duplicates(&packages).is_empty());
}

#[test]
fn two_versions_of_one_crate_are_reported_with_both() {
    let packages = vec![
        package("winnow", "0.7.15", false, 1),
        package("winnow", "1.0.4", false, 1),
        package("serde", "1.0.0", true, 1),
    ];

    assert_eq!(
        find_duplicates(&packages),
        vec![DuplicateVersions {
            name: "winnow".to_owned(),
            versions: vec!["0.7.15".to_owned(), "1.0.4".to_owned()],
        }]
    );
}

#[test]
fn the_heaviest_direct_dependencies_come_back_ranked() {
    let report = DependencyReport {
        packages: vec![
            package("light", "1.0.0", true, 1),
            package("heavy", "1.0.0", true, 40),
            package("indirect", "1.0.0", false, 90),
        ],
        ..DependencyReport::default()
    };

    let ranked: Vec<&str> = report
        .heaviest_direct()
        .iter()
        .map(|package| package.name.as_str())
        .collect();

    assert_eq!(
        ranked,
        ["heavy", "light"],
        "only direct dependencies rank, heaviest first"
    );
}

#[test]
fn a_package_can_be_looked_up_by_identifier() {
    let report = DependencyReport {
        packages: vec![package("serde", "1.0.0", true, 1)],
        ..DependencyReport::default()
    };

    assert_eq!(
        report.package("serde@1.0.0").map(|found| found.name.as_str()),
        Some("serde")
    );
    assert!(report.package("absent@0.0.0").is_none());
}

#[test]
fn every_dependency_kind_has_a_label() {
    for kind in [
        DependencyKind::Normal,
        DependencyKind::Development,
        DependencyKind::Build,
    ] {
        assert!(!kind.label().is_empty());
    }
}
