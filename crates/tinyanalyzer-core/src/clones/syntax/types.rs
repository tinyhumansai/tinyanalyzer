//! The flattened syntax tree every clone detector reads.

/// What a leaf contributes to a normalized token stream.
///
/// The classes are the whole of the normalization: a detector never looks at a
/// leaf's text except through these, which is what makes "renamed" a property
/// of the class rather than something each detector reimplements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// A node with children.
    Inner,
    /// A value or field name: renamed copies differ here.
    Ident,
    /// A type name: kept by type-shape hashing, blinded by code hashing.
    TypeIdent,
    /// A literal, collapsed to its kind.
    Literal,
    /// A keyword, operator, punctuation, or primitive type: kept verbatim.
    Keyword,
}

/// Which field of its parent a node fills, for the fields the detector reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    /// No field, or one nothing here reads.
    Other,
    /// `name:` — the declared name of an item, field, or variant.
    Name,
    /// `parameters:` of a function.
    Parameters,
    /// `return_type:` of a function.
    ReturnType,
    /// `type:` — the self type of an impl, or the type of a field.
    Type,
    /// `trait:` of an impl.
    Trait,
}

/// One node of a flattened tree, stored in preorder.
///
/// A node's subtree is the index range `index..end`, its first child (if any)
/// is `index + 1`, and its next sibling is the node at `end`. Storing the tree
/// that way costs one `u32` per node instead of a child vector, which matters
/// at two million nodes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Node {
    /// The grammar's name for the node kind.
    pub kind: &'static str,
    /// The field it fills in its parent.
    pub field: Field,
    /// How a leaf normalizes; [`Class::Inner`] for everything else.
    pub class: Class,
    /// Whether the grammar names the node (as opposed to anonymous punctuation).
    pub named: bool,
    /// Whether the node is test code: a `#[test]` function, a `#[cfg(test)]`
    /// module, or anything inside one.
    pub test: bool,
    /// Whether the parser recovered from an error inside this subtree.
    pub error: bool,
    /// Index of the parent, or `u32::MAX` for the root.
    pub parent: u32,
    /// One past the last node of this subtree.
    pub end: u32,
    /// Index of the first leaf of this subtree in [`Tree::leaves`].
    pub first_leaf: u32,
    /// Number of leaves in this subtree.
    pub leaf_count: u32,
    /// Byte offset of the first byte.
    pub start_byte: u32,
    /// Byte offset one past the last byte.
    pub end_byte: u32,
    /// One-based line of the first byte.
    pub start_line: u32,
    /// One-based line of the last byte.
    pub end_line: u32,
    /// Identifier-blind structural hash of the subtree; see [`super::shape`].
    pub shape: u64,
}

/// One parsed file, flattened.
#[derive(Debug)]
pub(crate) struct Tree<'s> {
    /// The text the tree was parsed from.
    pub source: &'s str,
    /// Every kept node, in preorder.
    pub nodes: Vec<Node>,
    /// Leaf node indices, in source order.
    pub leaves: Vec<u32>,
}
