//! The symbol index: every item reduced to its signature, its shape, and the
//! names it touches.
//!
//! Where clone groups answer "what is written twice", the index answers the
//! questions around it: which functions take the same parameter types and
//! return the same thing, which items mention the same set of types, which
//! shapes recur under different names. It is one record per item, written as
//! JSON lines by `tinyanalyzer --output symbols`, and meant to be filtered,
//! joined, and grouped by whatever reads it.

use crate::clones::detect::Parsed;
use crate::clones::syntax::{Class, Field, Tree};
use crate::clones::types::SymbolRecord;
use std::collections::BTreeSet;

/// Node kinds that become a record.
const ITEMS: &[&str] = &[
    "function_item",
    "function_signature_item",
    "struct_item",
    "enum_item",
    "union_item",
    "trait_item",
    "impl_item",
    "const_item",
    "static_item",
    "type_item",
    "mod_item",
    "macro_definition",
];

/// Every item in every file, in file and line order.
#[must_use]
pub(crate) fn index(files: &[Parsed<'_>]) -> Vec<SymbolRecord> {
    files
        .iter()
        .flat_map(|parsed| {
            let tree = &parsed.tree;
            (0..u32::try_from(tree.nodes.len()).unwrap_or(u32::MAX))
                .filter(|&node| ITEMS.contains(&tree.nodes[node as usize].kind))
                .filter_map(move |node| record(parsed, node))
        })
        .collect()
}

/// One item's record.
fn record(parsed: &Parsed<'_>, node: u32) -> Option<SymbolRecord> {
    let tree = &parsed.tree;
    let data = &tree.nodes[node as usize];
    let qualified_name = tree.qualified_name(node)?;
    let name = qualified_name
        .rsplit("::")
        .next()
        .unwrap_or(&qualified_name)
        .to_owned();

    Some(SymbolRecord {
        kind: data.kind.to_owned(),
        name,
        qualified_name,
        file: parsed.input.path.to_owned(),
        start_line: data.start_line as usize,
        end_line: data.end_line as usize,
        editable: parsed.input.editable,
        is_test: parsed.input.is_test_path || data.test,
        shape: format!("{:016x}", data.shape),
        tokens: data.leaf_count as usize,
        parameters: parameters(tree, node),
        return_type: tree
            .field(node, Field::ReturnType)
            .map(|ty| tree.text(ty).to_owned()),
        types: names(tree, node, Class::TypeIdent),
        identifiers: names(tree, node, Class::Ident),
    })
}

/// Parameter types of a function, `self` receivers included as written.
fn parameters(tree: &Tree<'_>, node: u32) -> Vec<String> {
    let Some(list) = tree.field(node, Field::Parameters) else {
        return Vec::new();
    };
    tree.named_children(list)
        .map(|parameter| {
            tree.field(parameter, Field::Type)
                .map_or_else(|| tree.text(parameter), |ty| tree.text(ty))
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Every distinct leaf text of one class in a subtree, sorted.
fn names(tree: &Tree<'_>, node: u32, class: Class) -> Vec<String> {
    tree.leaves_of(node)
        .iter()
        .filter(|&&leaf| tree.nodes[leaf as usize].class == class)
        .map(|&leaf| tree.text(leaf))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod test;
