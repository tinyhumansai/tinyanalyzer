//! Unit tests for parsing and normalization.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Class, Field, NONE, Tree, hash_str, mix, parse};

fn tree(source: &str) -> Tree<'_> {
    parse(source).expect("the grammar loads")
}

fn find(tree: &Tree<'_>, kind: &str) -> u32 {
    (0..u32::try_from(tree.nodes.len()).unwrap())
        .find(|&node| tree.nodes[node as usize].kind == kind)
        .unwrap_or_else(|| panic!("no {kind} in the fixture"))
}

fn all(tree: &Tree<'_>, kind: &str) -> Vec<u32> {
    (0..u32::try_from(tree.nodes.len()).unwrap())
        .filter(|&node| tree.nodes[node as usize].kind == kind)
        .collect()
}

#[test]
fn comments_and_attributes_are_dropped() {
    let parsed = tree("/// docs\n#[inline]\nfn a() { // trailing\n    1 /* inner */ }\n");

    assert!(parsed.nodes.iter().all(|node| !node.kind.ends_with("comment")));
    assert!(parsed.nodes.iter().all(|node| node.kind != "attribute_item"));
}

#[test]
fn a_literal_is_one_leaf_whatever_its_structure() {
    let parsed = tree("fn a() { let s = \"two \\n parts\"; }\n");
    let literal = find(&parsed, "string_literal");

    assert_eq!(parsed.nodes[literal as usize].class, Class::Literal);
    assert_eq!(parsed.nodes[literal as usize].leaf_count, 1);
    assert_eq!(parsed.text(literal), "\"two \\n parts\"");
}

#[test]
fn leaves_are_classified_by_what_renaming_can_change() {
    let parsed = tree("fn a(x: Vec<u8>) -> u8 { x.len() as u8 }\n");
    let class_of = |kind: &str| parsed.nodes[find(&parsed, kind) as usize].class;

    assert_eq!(class_of("identifier"), Class::Ident);
    assert_eq!(class_of("field_identifier"), Class::Ident);
    assert_eq!(class_of("type_identifier"), Class::TypeIdent);
    assert_eq!(class_of("primitive_type"), Class::Keyword);
    assert_eq!(class_of("function_item"), Class::Inner);
}

#[test]
fn renamed_copies_share_a_shape_and_differ_exactly() {
    let parsed = tree(
        "fn a(x: u8) -> u8 { let y = x + 1; y * 2 }\nfn b(p: u8) -> u8 { let q = p + 1; q * 2 }\n",
    );
    let functions = all(&parsed, "function_item");
    let (a, b) = (functions[0], functions[1]);

    assert_eq!(parsed.nodes[a as usize].shape, parsed.nodes[b as usize].shape);
    assert_ne!(parsed.exact_hash(&[a]), parsed.exact_hash(&[b]));
}

#[test]
fn a_changed_operator_changes_the_shape() {
    let parsed = tree("fn a(x: u8) -> u8 { x + 1 }\nfn b(x: u8) -> u8 { x - 1 }\n");
    let functions = all(&parsed, "function_item");

    assert_ne!(
        parsed.nodes[functions[0] as usize].shape,
        parsed.nodes[functions[1] as usize].shape
    );
}

#[test]
fn independent_statements_hash_in_any_order() {
    let parsed = tree(
        "fn a() { left.push(1); right.push(2); }\nfn b() { right.push(2); left.push(1); }\n",
    );
    let blocks = all(&parsed, "block");

    assert_eq!(
        parsed.nodes[blocks[0] as usize].shape,
        parsed.nodes[blocks[1] as usize].shape
    );
}

#[test]
fn dependent_statements_keep_their_order() {
    let parsed = tree(
        "fn a() { x.push(1); x.clear(); }\nfn b() { x.clear(); x.push(1); }\n",
    );
    let blocks = all(&parsed, "block");

    assert_ne!(
        parsed.nodes[blocks[0] as usize].shape,
        parsed.nodes[blocks[1] as usize].shape
    );
}

#[test]
fn a_let_starts_a_new_run() {
    let parsed = tree(
        "fn a() { let v = 1; other(); }\nfn b() { other(); let v = 1; }\n",
    );
    let blocks = all(&parsed, "block");

    assert_ne!(
        parsed.nodes[blocks[0] as usize].shape,
        parsed.nodes[blocks[1] as usize].shape
    );
}

#[test]
fn test_attributes_mark_their_item_and_everything_inside() {
    let parsed = tree(
        "fn shipped() {}\n#[cfg(test)]\nmod tests {\n    fn helper() {}\n}\n#[tokio::test]\nasync fn live() {}\n",
    );
    let functions = all(&parsed, "function_item");
    let test_of = |node: u32| parsed.nodes[node as usize].test;

    assert!(!test_of(functions[0]));
    assert!(test_of(functions[1]));
    assert!(test_of(functions[2]));
}

#[test]
fn an_inner_test_attribute_marks_the_whole_file() {
    let parsed = tree("#![cfg(test)]\nfn helper() {}\n");

    assert!(parsed.nodes.iter().all(|node| node.test));
}

#[test]
fn syntax_errors_are_flagged_rather_than_fatal() {
    let parsed = tree("fn broken( { let = ; }\nfn fine() {}\n");

    assert!(parsed.nodes[0].error);
    let functions = all(&parsed, "function_item");
    assert!(functions.iter().any(|&node| !parsed.nodes[node as usize].error));
}

#[test]
fn an_empty_file_is_a_tree_with_one_node() {
    let parsed = tree("");

    assert_eq!(parsed.nodes.len(), 1);
    assert_eq!(parsed.leaves.len(), 0);
    assert_eq!(parsed.nodes[0].parent, NONE);
}

#[test]
fn the_flattened_tree_is_navigable() {
    let parsed = tree("impl Parser { fn advance(&mut self, by: usize) -> usize { by } }\n");
    let function = find(&parsed, "function_item");
    let impl_item = find(&parsed, "impl_item");

    assert!(parsed.contains(impl_item, function));
    assert!(!parsed.contains(function, impl_item));
    assert_eq!(parsed.name(function), Some("advance"));
    assert_eq!(
        parsed.qualified_name(function),
        Some("Parser::advance".to_owned())
    );
    assert_eq!(parsed.qualified_name(impl_item), Some("Parser".to_owned()));
    let parameters = parsed.field(function, Field::Parameters).unwrap();
    assert_eq!(parsed.named_children(parameters).count(), 2);
    assert_eq!(
        parsed.text(parsed.field(function, Field::ReturnType).unwrap()),
        "usize"
    );
    assert_eq!(parsed.enclosing(function, &["impl_item"]), Some(impl_item));
    assert_eq!(parsed.enclosing(impl_item, &["function_item"]), None);
}

#[test]
fn trait_impls_and_trait_methods_are_qualified() {
    let parsed = tree(
        "impl Display for Point { fn fmt(&self) {} }\ntrait Shape { fn area(&self) -> f64; }\n",
    );
    let method = find(&parsed, "function_item");
    let signature = find(&parsed, "function_signature_item");

    assert_eq!(
        parsed.qualified_name(method),
        Some("Display for Point::fmt".to_owned())
    );
    assert_eq!(
        parsed.qualified_name(signature),
        Some("Shape::area".to_owned())
    );
}

#[test]
fn value_names_exclude_fields_and_types() {
    let parsed = tree("fn a(total: Count) { total.value += step; }\n");
    let block = find(&parsed, "block");
    let names: Vec<&str> = parsed.value_names(block).into_iter().collect();

    assert_eq!(names, ["step", "total"]);
}

#[test]
fn identifiers_share_a_token_and_keywords_do_not() {
    let parsed = tree("fn a() { x + y - 1 }\n");
    let token_of = |kind: &str| parsed.token(find(&parsed, kind));
    let plus = parsed
        .leaves
        .iter()
        .copied()
        .find(|&leaf| parsed.text(leaf) == "+")
        .unwrap();
    let minus = parsed
        .leaves
        .iter()
        .copied()
        .find(|&leaf| parsed.text(leaf) == "-")
        .unwrap();

    assert_eq!(token_of("identifier"), 1);
    assert_ne!(parsed.token(plus), parsed.token(minus));
    assert!(parsed.token(plus) >= 2 && parsed.token(plus) < 1 << 31);
}

#[test]
fn hashing_is_stable_and_order_sensitive() {
    assert_eq!(hash_str("abc"), hash_str("abc"));
    assert_ne!(hash_str("abc"), hash_str("acb"));
    assert_ne!(mix(1, 2), mix(2, 1));
    assert_eq!(mix(7, 9), mix(7, 9));
}
