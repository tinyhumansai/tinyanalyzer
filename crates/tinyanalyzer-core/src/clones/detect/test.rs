//! Unit tests for the detection layers and the merge.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::similar::{self, dice, edit_distance, estimated_jaccard, kind_bag, ratio, signature};
use super::suffix::{lcp_array, repeats, suffix_array};
use super::{Candidate, Limits, Parsed, Unit, fragment_kind, merge, run, subsumes, tidy, trim};
use crate::clones::syntax::parse;
use crate::clones::types::{CloneInput, CloneKind, Detector, FragmentKind};
use std::collections::BTreeSet;

fn parsed<'s>(files: &[(&'s str, &'s str)]) -> Vec<Parsed<'s>> {
    files
        .iter()
        .map(|&(path, text)| Parsed {
            input: CloneInput {
                path,
                text,
                is_test_path: false,
                editable: true,
            },
            tree: parse(text).expect("the grammar loads"),
        })
        .collect()
}

fn limits(min_tokens: u32, min_lines: u32) -> Limits {
    Limits {
        min_tokens,
        min_lines,
        similarity: 0.85,
        kinds: FragmentKind::ALL.into_iter().collect(),
    }
}

fn naive_suffix_array(text: &[u32]) -> Vec<u32> {
    let mut positions: Vec<u32> = (0..u32::try_from(text.len()).unwrap()).collect();
    positions.sort_by(|&a, &b| text[a as usize..].cmp(&text[b as usize..]));
    positions
}

/// Statement shapes that do not repeat each other, so a body built from them
/// is long enough to report without being repetitive inside itself.
const SHAPES: [&str; 12] = [
    "let {v} = source.read(offset)?;",
    "if {v}.is_empty() {{ return Err(Error::Empty); }}",
    "for item in {v}.iter() {{ total += item.len(); }}",
    "let {v} = match kind {{ Kind::A => 1, Kind::B => 2, _ => 0 }};",
    "while let Some(next) = queue.pop() {{ seen.insert(next); }}",
    "{v}.sort_by(|a, b| b.cmp(a));",
    "let {v}: Vec<String> = names.iter().map(ToString::to_string).collect();",
    "assert!({v} > limit, \"too small\");",
    "out.push_str(&format!(\"{{}}\", {v}));",
    "let {v} = Config {{ depth: 3, wide: true }};",
    "loop {{ if tick() {{ break; }} }}",
    "{v} = {v}.wrapping_mul(31) ^ salt;",
];

/// A function body of `count` distinct statements, binding names from `seed`.
fn body(seed: &str, count: usize) -> String {
    (0..count)
        .map(|index| {
            format!(
                "    {}\n",
                SHAPES[index % SHAPES.len()].replace("{v}", &format!("{seed}{index}"))
            )
            .replace("{{", "{")
            .replace("}}", "}")
        })
        .collect()
}

#[test]
fn the_suffix_array_matches_a_brute_force_sort() {
    let inputs: [&[u32]; 4] = [
        &[3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 1 << 31],
        &[7, 7, 7, 7, 7, 7, 1 << 31],
        &[2, 1, 2, 1, 2, (1 << 31) + 1, 2, 1, 2, 1 << 31],
        &[],
    ];
    for text in inputs {
        assert_eq!(suffix_array(text), naive_suffix_array(text));
    }
}

#[test]
fn the_lcp_array_measures_adjacent_suffixes() {
    let text = [1, 2, 1, 2, 3, 1 << 31];
    let sa = suffix_array(&text);
    let lcp = lcp_array(&text, &sa);

    for index in 1..sa.len() {
        let (left, right) = (sa[index - 1] as usize, sa[index] as usize);
        let common = text[left..]
            .iter()
            .zip(&text[right..])
            .take_while(|(a, b)| a == b)
            .count();
        assert_eq!(lcp[index] as usize, common);
    }
}

#[test]
fn repeats_report_left_maximal_runs_only() {
    // "5 1 2 3" appears twice; "1 2 3" alone is its tail and is not reported.
    let text = [5, 1, 2, 3, 9, 5, 1, 2, 3, 8, 1 << 31];
    let sa = suffix_array(&text);
    let lcp = lcp_array(&text, &sa);
    let found = repeats(&text, &sa, &lcp, 3, 64);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].length, 4);
    assert_eq!(found[0].starts, [0, 5]);
}

#[test]
fn repeats_below_the_minimum_are_not_reported() {
    let text = [5, 1, 2, 3, 9, 5, 1, 2, 3, 8, 1 << 31];
    let sa = suffix_array(&text);
    let lcp = lcp_array(&text, &sa);

    assert_eq!(repeats(&text, &sa, &lcp, 5, 64).len(), 0);
}

#[test]
fn repeats_cap_their_occurrences() {
    let mut text: Vec<u32> = [4, 5, 6].repeat(10);
    text.push(1 << 31);
    let sa = suffix_array(&text);
    let lcp = lcp_array(&text, &sa);
    let found = repeats(&text, &sa, &lcp, 3, 2);

    assert!(found.iter().all(|repeat| repeat.starts.len() <= 2));
}

#[test]
fn trimming_keeps_complete_statements() {
    let files = parsed(&[("a.rs", "fn f() { a(1); b(2); c(3); }\n")]);
    let tree = &files[0].tree;
    // Leaves: fn f ( ) { a ( 1 ) ; b ( 2 ) ; c ( 3 ) ; }
    // From the `1` of the first call to the `;` after `c(3)`.
    let first = tree
        .leaves
        .iter()
        .position(|&leaf| tree.text(leaf) == "1")
        .unwrap();
    let last = tree.leaves.len() - 2;
    let nodes = trim(tree, first, last).expect("two whole statements fit");

    let texts: Vec<&str> = nodes.iter().map(|&node| tree.text(node)).collect();
    assert_eq!(texts, ["b(2);", "c(3);"]);
}

#[test]
fn trimming_an_exact_node_returns_it() {
    let files = parsed(&[("a.rs", "fn f() { a(1); }\n")]);
    let tree = &files[0].tree;
    let nodes = trim(tree, 0, tree.leaves.len() - 1).unwrap();

    assert_eq!(tree.nodes[nodes[0] as usize].kind, "function_item");
}

#[test]
fn trimming_inside_an_expression_gives_nothing() {
    let files = parsed(&[("a.rs", "fn f() { let x = (a + b) * (c + d); }\n")]);
    let tree = &files[0].tree;
    let plus = tree
        .leaves
        .iter()
        .position(|&leaf| tree.text(leaf) == "b")
        .unwrap();

    assert_eq!(trim(tree, plus, plus + 3), None);
}

#[test]
fn fragments_are_recognized_by_their_place() {
    let source = "fn f() { if a { b } else if c { d } for x in y { z } let g = |v| v; match m { _ => 1 } }\nstruct S { a: u8 }\nimpl S {}\ntrait T {}\n";
    let files = parsed(&[("a.rs", source)]);
    let tree = &files[0].tree;
    let kinds: BTreeSet<FragmentKind> = (0..u32::try_from(tree.nodes.len()).unwrap())
        .filter_map(|node| fragment_kind(tree, node))
        .collect();

    assert_eq!(
        kinds,
        FragmentKind::ALL
            .into_iter()
            .filter(|kind| *kind != FragmentKind::Statements)
            .collect()
    );

    // The `else if` is part of the chain, not a second one.
    let chains = (0..u32::try_from(tree.nodes.len()).unwrap())
        .filter(|&node| fragment_kind(tree, node) == Some(FragmentKind::IfChain))
        .count();
    assert_eq!(chains, 1);
}

#[test]
fn renamed_functions_form_one_group() {
    let a = format!(
        "fn first(source: Reader, offset: u64) {{\n{}}}\n",
        body("x", 8)
    );
    let b = format!(
        "fn second(input: Reader, base: u64) {{\n{}}}\n",
        body("y", 8)
    )
    .replace("source", "input")
    .replace("offset", "base");
    let files = parsed(&[("a.rs", &a), ("b.rs", &b)]);
    let groups = run(&files, &limits(30, 4));

    let function = groups
        .iter()
        .find(|group| group.fragment == FragmentKind::Function)
        .expect("the two functions are one group");
    assert_eq!(function.units.len(), 2);
    assert_eq!(function.kind, CloneKind::Renamed);
    assert!(function.detectors.contains(&Detector::SubtreeHash));
    // Nothing smaller inside them survives on its own.
    assert_eq!(groups.len(), 1);
}

#[test]
fn identical_functions_are_exact() {
    let a = format!(
        "fn first(source: Reader, offset: u64) {{\n{}}}\n",
        body("x", 8)
    );
    let files = parsed(&[("a.rs", &a), ("b.rs", &a)]);
    let groups = run(&files, &limits(30, 4));

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].kind, CloneKind::Exact);
}

#[test]
fn reordered_independent_statements_still_match() {
    let a = "fn first() {\n    alpha.push(1);\n    beta.insert(2, 3);\n    gamma.extend([4, 5]);\n    delta.clear();\n    epsilon.sort();\n}\n";
    let b = "fn second() {\n    beta.insert(2, 3);\n    alpha.push(1);\n    gamma.extend([4, 5]);\n    epsilon.sort();\n    delta.clear();\n}\n";
    let files = parsed(&[("a.rs", a), ("b.rs", b)]);
    let groups = run(&files, &limits(20, 4));

    let function = groups
        .iter()
        .find(|group| group.fragment == FragmentKind::Function)
        .expect("reordering does not hide the copy");
    assert_eq!(function.kind, CloneKind::NearMiss);
    assert!(function.detectors.contains(&Detector::Reordered));
}

#[test]
fn a_run_of_statements_inside_larger_functions_is_found() {
    let shared = body("v", 8);
    let a = format!("fn first() {{\n    setup_one();\n{shared}    finish_one(1, 2, 3);\n}}\n");
    let b = format!(
        "fn second(flag: bool) {{\n    if flag {{ return; }}\n    other_setup(9);\n{shared}    done();\n}}\n"
    );
    let files = parsed(&[("a.rs", &a), ("b.rs", &b)]);
    // Strict enough that the two functions are not near-copies of each other,
    // which would otherwise swallow the shared run.
    let mut limits = limits(40, 4);
    limits.similarity = 0.99;
    let groups = run(&files, &limits);

    let run_group = groups
        .iter()
        .find(|group| group.fragment == FragmentKind::Statements)
        .expect("the shared statements are found");
    assert!(run_group.detectors.contains(&Detector::SuffixArray));
    assert_eq!(run_group.units.len(), 2);
    assert_eq!(run_group.units[0].nodes.len(), 8);
}

#[test]
fn a_near_miss_with_an_extra_statement_is_found() {
    let a = format!(
        "fn first(source: Reader, offset: u64) {{\n{}}}\n",
        body("x", 10)
    );
    let b = format!(
        "fn second(source: Reader, offset: u64) {{\n{}    log(offset);\n}}\n",
        body("x", 10)
    );
    let files = parsed(&[("a.rs", &a), ("b.rs", &b)]);
    let mut limits = limits(30, 4);
    limits.kinds.remove(&FragmentKind::Statements);
    let groups = run(&files, &limits);

    let near = groups
        .iter()
        .find(|group| group.kind == CloneKind::NearMiss)
        .expect("one statement does not hide the copy");
    assert!(near.detectors.contains(&Detector::MinHash));
    assert!(near.detectors.contains(&Detector::EditDistance));
    assert!(near.similarity >= 0.85 && near.similarity < 1.0);
}

#[test]
fn large_near_misses_are_confirmed_without_edit_distance() {
    let a = format!(
        "fn first(source: Reader, offset: u64) {{\n{}}}\n",
        body("x", 40)
    );
    let b = format!(
        "fn second(source: Reader, offset: u64) {{\n{}    log(offset);\n}}\n",
        body("x", 40)
    );
    let files = parsed(&[("a.rs", &a), ("b.rs", &b)]);
    let mut limits = limits(30, 4);
    limits.kinds = BTreeSet::from([FragmentKind::Function]);
    let groups = run(&files, &limits);

    let near = groups
        .iter()
        .find(|group| group.kind == CloneKind::NearMiss)
        .expect("the large pair is confirmed");
    assert!(!near.detectors.contains(&Detector::EditDistance));
}

#[test]
fn unrelated_functions_are_not_grouped() {
    let a = format!(
        "fn first(source: Reader, offset: u64) {{\n{}}}\n",
        body("x", 8)
    );
    let b = "fn second() {\n    match state { Ready => go(), Waiting(n) if n > 3 => wait(n), _ => {} }\n    while let Some(item) = queue.pop() { handle(item)?; }\n    loop { break; }\n}\n";
    let files = parsed(&[("a.rs", &a), ("b.rs", b)]);

    assert_eq!(run(&files, &limits(20, 3)).len(), 0);
}

#[test]
fn fragments_below_the_thresholds_are_ignored() {
    let a = format!(
        "fn first(source: Reader, offset: u64) {{\n{}}}\n",
        body("x", 3)
    );
    let files = parsed(&[("a.rs", &a), ("b.rs", &a)]);
    let tokens = files[0].tree.nodes[1].leaf_count;

    assert_eq!(run(&files, &limits(tokens + 1, 1)).len(), 0);
    assert_eq!(run(&files, &limits(tokens, 1)).len(), 1);
    assert_eq!(run(&files, &limits(1, 6)).len(), 0);
}

#[test]
fn type_shapes_compare_field_types_and_blind_names() {
    let a = "struct Location {\n    file: String,\n    line: usize,\n    column: usize,\n    note: Option<String>,\n}\n";
    let b = "struct Position {\n    path: String,\n    row: usize,\n    col: usize,\n    hint: Option<String>,\n}\n";
    let c = "struct Other {\n    path: String,\n    row: u32,\n    col: usize,\n    hint: Option<String>,\n}\n";
    let files = parsed(&[("a.rs", a), ("b.rs", b), ("c.rs", c)]);
    let groups = run(&files, &limits(1000, 5));

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].fragment, FragmentKind::TypeShape);
    assert_eq!(groups[0].units.len(), 2);
    assert_eq!(groups[0].kind, CloneKind::Renamed);
}

#[test]
fn syntax_errors_keep_a_fragment_out() {
    let a = format!(
        "fn first(source: Reader) {{\n{}    let = ;\n}}\n",
        body("x", 8)
    );
    let files = parsed(&[("a.rs", &a), ("b.rs", &a)]);

    assert!(
        run(&files, &limits(30, 4))
            .iter()
            .all(|group| group.fragment != FragmentKind::Function)
    );
}

fn candidate(units: Vec<Unit>) -> Candidate {
    Candidate {
        units,
        fragment: FragmentKind::Statements,
        kind: CloneKind::Exact,
        detectors: BTreeSet::from([Detector::SuffixArray]),
        similarity: 1.0,
        shape: 0,
        recursive: false,
    }
}

fn node_of(files: &[Parsed<'_>], file: usize, text: &str) -> u32 {
    let tree = &files[file].tree;
    (0..u32::try_from(tree.nodes.len()).unwrap())
        .find(|&node| tree.nodes[node as usize].named && tree.text(node) == text)
        .unwrap_or_else(|| panic!("no node spells {text}"))
}

#[test]
fn a_copy_inside_another_copy_marks_the_group_recursive() {
    let files = parsed(&[("a.rs", "fn f() { outer(inner(1)); }\n")]);
    let outer = node_of(&files, 0, "outer(inner(1))");
    let inner = node_of(&files, 0, "inner(1)");
    let mut group = candidate(vec![
        Unit {
            file: 0,
            nodes: vec![inner],
        },
        Unit {
            file: 0,
            nodes: vec![outer],
        },
    ]);
    tidy(&files, &mut group);

    assert!(group.recursive);
    assert_eq!(group.units.len(), 1);
}

#[test]
fn overlapping_copies_keep_the_first() {
    let files = parsed(&[("a.rs", "fn f() { a(); b(); c(); }\n")]);
    let tree = &files[0].tree;
    let statements: Vec<u32> = (0..u32::try_from(tree.nodes.len()).unwrap())
        .filter(|&node| tree.nodes[node as usize].kind == "expression_statement")
        .collect();
    let mut group = candidate(vec![
        Unit {
            file: 0,
            nodes: statements[1..3].to_vec(),
        },
        Unit {
            file: 0,
            nodes: statements[0..2].to_vec(),
        },
    ]);
    tidy(&files, &mut group);

    assert!(!group.recursive);
    assert_eq!(group.units.len(), 1);
    assert_eq!(group.units[0].nodes, statements[0..2]);
}

#[test]
fn merging_folds_duplicates_and_drops_groups_inside_bigger_ones() {
    let files = parsed(&[
        ("a.rs", "fn f() { work(1); }\n"),
        ("b.rs", "fn g() { work(1); }\n"),
    ]);
    let whole = |file: usize| Unit {
        file: u32::try_from(file).unwrap(),
        nodes: vec![node_of(&files, file, "work(1);")],
    };
    let part = |file: usize| Unit {
        file: u32::try_from(file).unwrap(),
        nodes: vec![node_of(&files, file, "work(1)")],
    };

    let mut by_hash = candidate(vec![whole(0), whole(1)]);
    by_hash.detectors = BTreeSet::from([Detector::SubtreeHash]);
    by_hash.kind = CloneKind::Renamed;
    let merged = merge(
        &files,
        vec![
            candidate(vec![whole(0), whole(1)]),
            by_hash,
            candidate(vec![part(0), part(1)]),
            candidate(vec![whole(0)]),
        ],
    );

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].kind, CloneKind::Exact);
    assert_eq!(
        merged[0].detectors,
        BTreeSet::from([Detector::SuffixArray, Detector::SubtreeHash])
    );
}

#[test]
fn groups_covering_the_same_bytes_keep_the_earlier() {
    let files = parsed(&[("a.rs", "fn f() { x }\n"), ("b.rs", "fn g() { x }\n")]);
    let unit = |file: usize, text: &str| Unit {
        file: u32::try_from(file).unwrap(),
        nodes: vec![node_of(&files, file, text)],
    };
    let first = candidate(vec![unit(0, "{ x }"), unit(1, "{ x }")]);
    let second = candidate(vec![unit(0, "{ x }"), unit(1, "{ x }")]);
    let smaller = candidate(vec![unit(0, "x"), unit(1, "x")]);

    assert!(subsumes(&files, &first, 0, &second, 1));
    assert!(!subsumes(&files, &second, 1, &first, 0));
    assert!(subsumes(&files, &first, 0, &smaller, 2));
    assert!(!subsumes(&files, &smaller, 2, &first, 0));
}

#[test]
fn a_group_with_more_copies_is_not_subsumed_by_one_with_fewer() {
    let files = parsed(&[
        ("a.rs", "fn f() { x }\n"),
        ("b.rs", "fn g() { x }\n"),
        ("c.rs", "fn h() { x }\n"),
    ]);
    let unit = |file: usize, text: &str| Unit {
        file: u32::try_from(file).unwrap(),
        nodes: vec![node_of(&files, file, text)],
    };
    let few = candidate(vec![unit(0, "{ x }"), unit(1, "{ x }")]);
    let many = candidate(vec![unit(0, "x"), unit(1, "x"), unit(2, "x")]);

    assert!(!subsumes(&files, &few, 0, &many, 1));
}

#[test]
fn similarity_measures_behave_at_their_extremes() {
    let tokens: Vec<u32> = (0..40).collect();
    let same = signature(&tokens);
    assert!((estimated_jaccard(&same, &same) - 1.0).abs() < f64::EPSILON);
    let other: Vec<u32> = (1000..1040).collect();
    assert!(estimated_jaccard(&same, &signature(&other)) < 0.2);
    assert_eq!(
        similar::bands(&same).len(),
        similar::SIGNATURE / similar::BAND_ROWS
    );
    // Shorter than one shingle still has a signature.
    assert_ne!(signature(&[1, 2]), [u64::MAX; similar::SIGNATURE]);

    assert!((ratio(0, 0) - 1.0).abs() < f64::EPSILON);
    assert!((ratio(1, 4) - 0.25).abs() < f64::EPSILON);
}

#[test]
fn dice_and_edit_distance_agree_on_identical_trees() {
    let files = parsed(&[
        ("a.rs", "fn f() { a(1); }\n"),
        ("b.rs", "fn g() { b(2); }\n"),
    ]);
    let (a, b) = (&files[0].tree, &files[1].tree);

    assert!((dice(&kind_bag(a, 0), &kind_bag(b, 0)) - 1.0).abs() < f64::EPSILON);
    // Renamed leaves differ in text, not in label: no edits.
    assert_eq!(edit_distance(a, 0, b, 0), 0);
}

#[test]
fn edit_distance_counts_an_inserted_statement() {
    let files = parsed(&[
        ("a.rs", "fn f() { a(1); }\n"),
        ("b.rs", "fn f() { a(1); b; }\n"),
    ]);
    let (a, b) = (&files[0].tree, &files[1].tree);
    let added = b.nodes.len() - a.nodes.len();

    assert_eq!(edit_distance(a, 0, b, 0), added);
    assert_eq!(edit_distance(b, 0, a, 0), added);
}
