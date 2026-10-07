//! Unit tests for anti-unification and the choice of shared code.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{camel, identifier, shorten, sketch};
use crate::clones::detect::{Candidate, Parsed, Unit};
use crate::clones::syntax::parse;
use crate::clones::types::{CloneInput, CloneKind, FragmentKind, ParameterKind, SketchKind};
use std::collections::BTreeSet;

fn parsed<'s>(files: &[&'s str]) -> Vec<Parsed<'s>> {
    files
        .iter()
        .map(|&text| Parsed {
            input: CloneInput {
                path: "a.rs",
                text,
                is_test_path: false,
                editable: true,
            },
            tree: parse(text).expect("the grammar loads"),
        })
        .collect()
}

/// Every top-level node of `kind`, one unit each, across the files.
fn units_of(files: &[Parsed<'_>], kind: &str) -> Vec<Unit> {
    files
        .iter()
        .enumerate()
        .flat_map(|(file, parsed)| {
            let tree = &parsed.tree;
            (0..u32::try_from(tree.nodes.len()).unwrap())
                .filter(|&node| tree.nodes[node as usize].kind == kind)
                .map(move |node| Unit {
                    file: u32::try_from(file).unwrap(),
                    nodes: vec![node],
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn group(units: Vec<Unit>, fragment: FragmentKind) -> Candidate {
    Candidate {
        units,
        fragment,
        kind: CloneKind::Renamed,
        detectors: BTreeSet::new(),
        similarity: 1.0,
        shape: 0,
        recursive: false,
    }
}

#[test]
fn identical_copies_need_no_parameters() {
    let files = parsed(&[
        "fn load() -> u8 { read(1) + 2 }\n",
        "fn fetch() -> u8 { read(1) + 2 }\n",
    ]);
    let result = sketch(
        &files,
        &group(units_of(&files, "function_item"), FragmentKind::Function),
    );

    assert_eq!(result.kind, SketchKind::Function);
    assert_eq!(result.parameters.len(), 0);
    assert_eq!(result.signature, "fn shared_load() -> u8");
    assert!(result.summary.contains("identical"));
}

#[test]
fn differing_literals_become_typed_parameters() {
    let files = parsed(&[
        "fn a() { open(\"one\", 1, 1.5, true, 'x'); }\n",
        "fn b() { open(\"two\", 2, 2.5, false, 'y'); }\n",
    ]);
    let result = sketch(
        &files,
        &group(units_of(&files, "function_item"), FragmentKind::Function),
    );

    assert_eq!(result.kind, SketchKind::Function);
    assert!(
        result
            .parameters
            .iter()
            .all(|p| p.kind == ParameterKind::Literal)
    );
    assert_eq!(
        result.signature,
        "fn shared_a(p0: &str, p1: i64, p2: f64, p3: bool, p4: char)"
    );
    assert_eq!(result.parameters[0].values, ["\"one\"", "\"two\""]);
    assert_eq!(result.parameters[0].line, 1);
}

#[test]
fn a_consistently_renamed_variable_is_one_parameter() {
    let files = parsed(&[
        "fn a() { let n = make(); use_it(n); keep(n); }\n",
        "fn b() { let m = make(); use_it(m); keep(m); }\n",
    ]);
    let result = sketch(
        &files,
        &group(units_of(&files, "function_item"), FragmentKind::Function),
    );

    assert_eq!(result.parameters.len(), 1);
    assert_eq!(result.parameters[0].kind, ParameterKind::Identifier);
}

#[test]
fn types_alone_differing_suggest_a_generic() {
    let files = parsed(&[
        "fn a() -> Vec<Apple> { Vec::<Apple>::new() }\n",
        "fn b() -> Vec<Pear> { Vec::<Pear>::new() }\n",
    ]);
    let result = sketch(
        &files,
        &group(units_of(&files, "function_item"), FragmentKind::Function),
    );

    assert_eq!(result.kind, SketchKind::Generic);
    assert!(result.signature.starts_with("fn shared_a<T0>("));
}

#[test]
fn field_names_differing_suggest_a_macro() {
    let files = parsed(&[
        "fn a(s: &mut S) { s.width += 1; }\n",
        "fn b(s: &mut S) { s.height += 1; }\n",
    ]);
    let result = sketch(
        &files,
        &group(units_of(&files, "function_item"), FragmentKind::Function),
    );

    assert_eq!(result.kind, SketchKind::Macro);
    assert!(result.signature.starts_with("macro_rules! shared_a {"));
    assert!(result.signature.contains(":ident"));
}

#[test]
fn an_extra_statement_becomes_a_statements_parameter() {
    let files = parsed(&[
        "fn a() { one(); two(); three(); }\n",
        "fn b() { one(); two(); extra(); three(); }\n",
    ]);
    let result = sketch(
        &files,
        &group(units_of(&files, "function_item"), FragmentKind::Function),
    );

    assert!(
        result
            .parameters
            .iter()
            .any(|parameter| parameter.kind == ParameterKind::Statements)
    );
    assert!(result.signature.contains("impl FnOnce()"));
}

#[test]
fn adjacent_copies_differing_in_values_become_a_loop() {
    let files = parsed(&["fn f() { add(\"a\", 1); add(\"b\", 2); add(\"c\", 3); }\n"]);
    let result = sketch(
        &files,
        &group(
            units_of(&files, "expression_statement"),
            FragmentKind::Statements,
        ),
    );

    assert_eq!(result.kind, SketchKind::Loop);
    assert_eq!(
        result.signature,
        "for (p0, p1) in [(\"a\", 1), (\"b\", 2), (\"c\", 3)] { … }"
    );
}

#[test]
fn a_loop_over_one_value_binds_it_directly() {
    let files = parsed(&["fn f() { add(1); add(2); add(3); add(4); add(5); }\n"]);
    let result = sketch(
        &files,
        &group(
            units_of(&files, "expression_statement"),
            FragmentKind::Statements,
        ),
    );

    assert_eq!(result.kind, SketchKind::Loop);
    assert_eq!(result.signature, "for p0 in [1, 2, 3, 4, …] { … }");
}

#[test]
fn separated_copies_are_not_a_loop() {
    let files = parsed(&["fn f() { add(1); other(); add(2); add(3); }\n"]);
    let units: Vec<Unit> = units_of(&files, "expression_statement")
        .into_iter()
        .filter(|unit| files[0].tree.text(unit.first()).starts_with("add"))
        .collect();
    let result = sketch(&files, &group(units, FragmentKind::Statements));

    assert_eq!(result.kind, SketchKind::Function);
    assert_eq!(result.signature, "fn f_step(p0: i64)");
}

#[test]
fn a_recursive_group_suggests_a_recursive_helper() {
    let files = parsed(&["fn a() { x(1) }\n", "fn b() { x(2) }\n"]);
    let mut candidate = group(units_of(&files, "function_item"), FragmentKind::Function);
    candidate.recursive = true;
    let result = sketch(&files, &candidate);

    assert_eq!(result.kind, SketchKind::Recursive);
    assert!(result.summary.contains("calls itself"));
}

#[test]
fn type_shapes_suggest_a_shared_type() {
    let files = parsed(&[
        "struct Location { file: String, line: usize }\n",
        "enum Kind { A(String), B(usize) }\nenum Sort { X(String), Y(usize) }\n",
    ]);
    let structs = sketch(
        &files,
        &group(
            [
                units_of(&files, "struct_item"),
                units_of(&files, "struct_item"),
            ]
            .concat(),
            FragmentKind::TypeShape,
        ),
    );
    let enums = sketch(
        &files,
        &group(units_of(&files, "enum_item"), FragmentKind::TypeShape),
    );

    assert_eq!(structs.kind, SketchKind::SharedType);
    assert_eq!(structs.signature, "struct SharedLocation { … }");
    assert_eq!(enums.signature, "enum SharedKind { … }");
}

#[test]
fn impl_blocks_are_named_for_their_type() {
    let files = parsed(&[
        "impl Graph<State> { fn len(&self) -> usize { 1 } }\n",
        "impl Tree<State> { fn len(&self) -> usize { 1 } }\n",
    ]);
    let result = sketch(
        &files,
        &group(units_of(&files, "impl_item"), FragmentKind::Impl),
    );

    assert!(result.signature.contains("shared_graph"));
}

#[test]
fn a_statement_run_outside_any_function_has_a_generic_name() {
    let files = parsed(&[
        "const A: u8 = 1;\nconst B: u8 = 2;\n",
        "const A: u8 = 3;\nconst B: u8 = 4;\n",
    ]);
    let units: Vec<Unit> = (0..2)
        .map(|file| {
            let tree = &files[file].tree;
            Unit {
                file: u32::try_from(file).unwrap(),
                nodes: tree.named_children(0).collect(),
            }
        })
        .collect();
    let result = sketch(&files, &group(units, FragmentKind::Statements));

    assert!(result.signature.contains("shared_helper"));
    assert!(result.summary.contains("2 places"));
}

#[test]
fn names_and_values_are_tidied_for_display() {
    assert_eq!(identifier("ParseConfig<T>"), "parse_config");
    assert_eq!(identifier("r#match"), "match");
    assert_eq!(identifier("<>"), "helper");
    assert_eq!(camel("shared_parse_config"), "SharedParseConfig");
    assert_eq!(shorten("a\n   b"), "a b");
    let long = "x".repeat(100);
    assert_eq!(shorten(&long).chars().count(), 48);
    assert!(shorten(&long).ends_with('…'));
}
