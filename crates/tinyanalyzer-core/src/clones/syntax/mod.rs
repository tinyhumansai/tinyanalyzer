//! Parsing a file with tree-sitter and flattening it into a normalized tree.
//!
//! Every clone detector reads the same [`Tree`]: comments and attributes are
//! dropped, literals are collapsed into one leaf each, every leaf is given a
//! [`Class`], and every node carries an identifier-blind structural hash. A
//! detector that wants something else from the source — the exact text of a
//! leaf, say — asks the tree for it rather than re-reading the file.
//!
//! The structural hash ([`Node::shape`]) is where two normalizations happen
//! that all the detectors share:
//!
//! - **Identifiers are blind.** Every value, field, and type name hashes the
//!   same, so `let total = a + b` and `let sum = x + y` are one shape.
//! - **Independent statements are unordered.** Inside a block, a run of
//!   adjacent statements that share no variable name is hashed in a canonical
//!   order, so two copies that set the same fields in a different order still
//!   match. "Share no variable name" is a def-use approximation, not a
//!   dependency analysis: two calls with side effects on different receivers
//!   are treated as independent.

mod types;

pub(crate) use types::{Class, Field, Node, Tree};

use std::collections::BTreeSet;

/// Marks the root's parent and other absent indices.
pub(crate) const NONE: u32 = u32::MAX;

/// Node kinds dropped from the tree entirely.
///
/// Comments and attributes change nothing about what code does, and a copy
/// that gained a doc comment is still a copy.
const TRIVIA: &[&str] = &[
    "line_comment",
    "block_comment",
    "attribute_item",
    "inner_attribute_item",
];

/// Node kinds kept as a single leaf, whatever their internal structure.
const LITERALS: &[&str] = &[
    "string_literal",
    "raw_string_literal",
    "char_literal",
    "integer_literal",
    "float_literal",
    "boolean_literal",
];

/// Leaf kinds that name a value or a field.
const IDENTS: &[&str] = &[
    "identifier",
    "field_identifier",
    "shorthand_field_identifier",
];

/// Mixes two 64-bit values into one, well distributed.
///
/// The `SplitMix64` finalizer over a rotated combination. Written out rather
/// than taken from `std::hash` because the hashes leave the process — a shape
/// hash is printed in the symbol index — and the standard library makes no
/// promise that its hasher is stable across releases.
#[must_use]
pub(crate) const fn mix(left: u64, right: u64) -> u64 {
    let mut value = left.rotate_left(23) ^ right.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Hashes a string with FNV-1a, then mixes it.
#[must_use]
pub(crate) fn hash_str(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    mix(hash, text.len() as u64)
}

/// Parses `source` as Rust and flattens it.
///
/// Returns `None` only when tree-sitter itself declines to produce a tree,
/// which happens when the grammar cannot be loaded. A file with syntax errors
/// still produces a tree; the damaged subtrees are flagged with
/// [`Node::error`] so no detector builds a clone out of them.
#[must_use]
pub(crate) fn parse(source: &str) -> Option<Tree<'_>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let parsed = parser.parse(source, None)?;

    let mut tree = flatten(source, &parsed);
    mark_tests(&mut tree);
    compute_shapes(&mut tree);
    Some(tree)
}

/// What the builder tracks for each node whose children it is visiting.
struct Frame {
    index: u32,
    /// Whether the previous sibling was a test attribute.
    pending_test: bool,
}

/// Walks the tree-sitter tree once, in preorder, into a [`Tree`].
fn flatten<'s>(source: &'s str, parsed: &tree_sitter::Tree) -> Tree<'s> {
    let mut tree = Tree {
        source,
        nodes: Vec::new(),
        leaves: Vec::new(),
    };
    let mut cursor = parsed.walk();
    let mut stack: Vec<Frame> = Vec::new();
    let mut root_pending = false;

    'walk: loop {
        let node = cursor.node();
        let kind = node.kind();

        if TRIVIA.contains(&kind) {
            let text = source.get(node.byte_range()).unwrap_or_default();
            if is_test_attribute(text) {
                if kind == "inner_attribute_item" {
                    if let Some(frame) = stack.last() {
                        tree.nodes[frame.index as usize].test = true;
                    }
                } else if let Some(frame) = stack.last_mut() {
                    frame.pending_test = true;
                } else {
                    root_pending = true;
                }
            }
        } else {
            let pending = stack.last_mut().map_or_else(
                || std::mem::take(&mut root_pending),
                |frame| std::mem::take(&mut frame.pending_test),
            );
            let index = push(&mut tree, &stack, &node, cursor.field_name(), pending);
            let is_literal = LITERALS.contains(&kind);
            if !is_literal && node.child_count() > 0 && cursor.goto_first_child() {
                stack.push(Frame {
                    index,
                    pending_test: false,
                });
                continue 'walk;
            }
            finish(&mut tree, index);
        }

        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
            if let Some(frame) = stack.pop() {
                finish(&mut tree, frame.index);
            }
        }
    }

    tree
}

/// Whether an attribute's text marks the item after it as test code.
fn is_test_attribute(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("cfg(test)") || compact.ends_with("test]")
}

/// Appends one node, before its children.
fn push(
    tree: &mut Tree<'_>,
    stack: &[Frame],
    node: &tree_sitter::Node<'_>,
    field: Option<&'static str>,
    test: bool,
) -> u32 {
    let kind = node.kind();
    // The root is a container even when the file is empty.
    let is_leaf = LITERALS.contains(&kind) || (node.child_count() == 0 && !stack.is_empty());
    let class = if !is_leaf {
        Class::Inner
    } else if LITERALS.contains(&kind) {
        Class::Literal
    } else if IDENTS.contains(&kind) {
        Class::Ident
    } else if kind == "type_identifier" {
        Class::TypeIdent
    } else {
        Class::Keyword
    };

    let index = to_u32(tree.nodes.len());
    let first_leaf = to_u32(tree.leaves.len());
    if is_leaf {
        tree.leaves.push(index);
    }

    tree.nodes.push(Node {
        kind,
        field: match field {
            Some("name") => Field::Name,
            Some("parameters") => Field::Parameters,
            Some("return_type") => Field::ReturnType,
            Some("type") => Field::Type,
            Some("trait") => Field::Trait,
            _ => Field::Other,
        },
        class,
        named: node.is_named(),
        test,
        error: node.has_error(),
        parent: stack.last().map_or(NONE, |frame| frame.index),
        end: index.saturating_add(1),
        first_leaf,
        leaf_count: 0,
        start_byte: to_u32(node.start_byte()),
        end_byte: to_u32(node.end_byte()),
        start_line: to_u32(node.start_position().row).saturating_add(1),
        end_line: to_u32(node.end_position().row).saturating_add(1),
        shape: 0,
    });

    index
}

/// Closes a node once its children are in: records where its subtree ends.
fn finish(tree: &mut Tree<'_>, index: u32) {
    let end = to_u32(tree.nodes.len());
    let leaves = to_u32(tree.leaves.len());
    let node = &mut tree.nodes[index as usize];
    node.end = end;
    node.leaf_count = leaves.saturating_sub(node.first_leaf);
}

/// Spreads the test flag from each marked node over its subtree.
fn mark_tests(tree: &mut Tree<'_>) {
    for index in 0..tree.nodes.len() {
        let parent = tree.nodes[index].parent;
        if parent != NONE && tree.nodes[parent as usize].test {
            tree.nodes[index].test = true;
        }
    }
}

/// Fills [`Node::shape`] bottom-up.
fn compute_shapes(tree: &mut Tree<'_>) {
    for index in (0..tree.nodes.len()).rev() {
        let node = tree.nodes[index];
        let kind = hash_str(node.kind);
        let shape = match node.class {
            Class::Inner => {
                let children: Vec<u32> = tree.children(to_u32(index)).collect();
                let hashes = if node.kind == "block" {
                    canonical_order(tree, &children)
                } else {
                    children
                        .iter()
                        .map(|&child| tree.nodes[child as usize].shape)
                        .collect()
                };
                hashes.into_iter().fold(kind, mix)
            }
            Class::Ident | Class::TypeIdent => mix(kind, 1),
            Class::Literal => mix(kind, 2),
            Class::Keyword => mix(kind, hash_str(tree.text(to_u32(index)))),
        };
        tree.nodes[index].shape = shape;
    }
}

/// The child hashes of a block, with independent statement runs sorted.
fn canonical_order(tree: &Tree<'_>, children: &[u32]) -> Vec<u64> {
    let mut hashes = Vec::with_capacity(children.len());
    let mut run: Vec<u64> = Vec::new();
    let mut run_names: BTreeSet<&str> = BTreeSet::new();

    for &child in children {
        let node = tree.nodes[child as usize];
        if !node.named {
            flush(&mut run, &mut hashes);
            run_names.clear();
            hashes.push(node.shape);
            continue;
        }

        let names = tree.value_names(child);
        if !run_names.is_disjoint(&names) || node.kind == "let_declaration" {
            flush(&mut run, &mut hashes);
            run_names.clear();
        }
        run.push(node.shape);
        run_names.extend(names);
    }
    flush(&mut run, &mut hashes);

    hashes
}

/// Emits a run of independent statements in canonical order.
fn flush(run: &mut Vec<u64>, out: &mut Vec<u64>) {
    run.sort_unstable();
    out.append(run);
}

/// Narrows a length or offset to the `u32` the tree stores.
///
/// Saturates rather than wrapping: a source file past four gigabytes is not
/// parsed in the first place, since the walker's size limit stops it.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl<'s> Tree<'s> {
    /// The direct children of `index`, in order.
    pub(crate) fn children(&self, index: u32) -> impl Iterator<Item = u32> + '_ {
        let end = self.nodes[index as usize].end;
        let mut next = index.saturating_add(1);
        std::iter::from_fn(move || {
            if next >= end {
                return None;
            }
            let current = next;
            next = self.nodes[current as usize].end;
            Some(current)
        })
    }

    /// The named children of `index`, in order.
    pub(crate) fn named_children(&self, index: u32) -> impl Iterator<Item = u32> + '_ {
        self.children(index)
            .filter(|&child| self.nodes[child as usize].named)
    }

    /// The first child of `index` filling `field`.
    pub(crate) fn field(&self, index: u32, field: Field) -> Option<u32> {
        self.children(index)
            .find(|&child| self.nodes[child as usize].field == field)
    }

    /// The source text a node spans.
    pub(crate) fn text(&self, index: u32) -> &'s str {
        let node = &self.nodes[index as usize];
        self.source
            .get(node.start_byte as usize..node.end_byte as usize)
            .unwrap_or_default()
    }

    /// The leaf indices of a node's subtree, in source order.
    pub(crate) fn leaves_of(&self, index: u32) -> &[u32] {
        let node = &self.nodes[index as usize];
        let start = node.first_leaf as usize;
        let end = start.saturating_add(node.leaf_count as usize);
        self.leaves.get(start..end).unwrap_or_default()
    }

    /// Whether `inner` is `outer` or inside it.
    pub(crate) fn contains(&self, outer: u32, inner: u32) -> bool {
        inner >= outer && inner < self.nodes[outer as usize].end
    }

    /// Every value name (not field or type name) used in a subtree.
    pub(crate) fn value_names(&self, index: u32) -> BTreeSet<&'s str> {
        self.leaves_of(index)
            .iter()
            .filter(|&&leaf| self.nodes[leaf as usize].kind == "identifier")
            .map(|&leaf| self.text(leaf))
            .collect()
    }

    /// The normalized token code of one leaf.
    ///
    /// Identifiers and type names all map to `1`, literals to their kind, and
    /// keywords and punctuation to their text — the token alphabet a
    /// parameterized match is computed over. Codes stay below `2^31` so the
    /// suffix array can use the space above for per-file sentinels.
    pub(crate) fn token(&self, leaf: u32) -> u32 {
        let node = &self.nodes[leaf as usize];
        match node.class {
            Class::Ident | Class::TypeIdent => 1,
            _ => u32::try_from(node.shape % (1 << 31)).unwrap_or(2).max(2),
        }
    }

    /// Hashes the exact text of every leaf in a run of sibling nodes.
    ///
    /// Equal exact hashes mean a character-for-character copy, modulo
    /// whitespace, comments, and attributes.
    pub(crate) fn exact_hash(&self, nodes: &[u32]) -> u64 {
        nodes
            .iter()
            .flat_map(|&node| self.leaves_of(node).iter())
            .fold(0, |hash, &leaf| mix(hash, hash_str(self.text(leaf))))
    }

    /// The nearest ancestor-or-self of `index` whose kind is in `kinds`.
    pub(crate) fn enclosing(&self, index: u32, kinds: &[&str]) -> Option<u32> {
        let mut current = index;
        loop {
            if kinds.contains(&self.nodes[current as usize].kind) {
                return Some(current);
            }
            current = self.nodes[current as usize].parent;
            if current == NONE {
                return None;
            }
        }
    }

    /// The declared name of an item, if it has one.
    pub(crate) fn name(&self, index: u32) -> Option<&'s str> {
        self.field(index, Field::Name).map(|name| self.text(name))
    }

    /// A readable name for the item enclosing `index`: `Type::method` inside
    /// an impl, the bare name elsewhere.
    pub(crate) fn qualified_name(&self, index: u32) -> Option<String> {
        let item = self.enclosing(
            index,
            &[
                "function_item",
                "function_signature_item",
                "struct_item",
                "enum_item",
                "union_item",
                "trait_item",
                "impl_item",
                "mod_item",
                "const_item",
                "static_item",
                "type_item",
                "macro_definition",
            ],
        )?;
        let own = if self.nodes[item as usize].kind == "impl_item" {
            self.impl_name(item)
        } else {
            self.name(item).map(str::to_owned)
        }?;

        let parent = self.nodes[item as usize].parent;
        let owner = if parent == NONE {
            None
        } else {
            self.enclosing(parent, &["impl_item", "trait_item"])
        };
        Some(match owner {
            Some(owner) if self.nodes[owner as usize].kind == "impl_item" => {
                format!("{}::{own}", self.impl_name(owner).unwrap_or_default())
            }
            Some(owner) => format!("{}::{own}", self.name(owner).unwrap_or_default()),
            None => own,
        })
    }

    /// `Type` or `Trait for Type` for an impl block.
    fn impl_name(&self, item: u32) -> Option<String> {
        let ty = self.text(self.field(item, Field::Type)?);
        Some(match self.field(item, Field::Trait) {
            Some(tr) => format!("{} for {ty}", self.text(tr)),
            None => ty.to_owned(),
        })
    }
}

#[cfg(test)]
mod test;
