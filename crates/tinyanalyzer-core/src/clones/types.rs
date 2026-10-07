//! What clone detection reports.

use serde::{Deserialize, Serialize};

/// One file handed to clone detection.
#[derive(Debug, Clone, Copy)]
pub struct CloneInput<'a> {
    /// Path as reported: relative to the analysis root, with forward slashes.
    pub path: &'a str,
    /// The file's contents.
    pub text: &'a str,
    /// Whether the path alone marks this file as test code.
    pub is_test_path: bool,
    /// Whether a suggestion may ask for this file to change.
    ///
    /// Read-only code is still indexed, so a group can show that editable code
    /// re-implements something a read-only crate already has.
    pub editable: bool,
}

/// The granularity a clone was found at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FragmentKind {
    /// A whole function or method.
    Function,
    /// A whole impl block.
    Impl,
    /// A whole trait definition.
    Trait,
    /// A closure.
    Closure,
    /// One arm of a `match`.
    MatchArm,
    /// An `if` / `else if` chain.
    IfChain,
    /// A block that is the body of a loop or a branch.
    Block,
    /// A run of consecutive statements or items, found by the suffix array.
    Statements,
    /// A struct or enum definition, compared by field types in order.
    TypeShape,
}

impl FragmentKind {
    /// Every kind, in the order the default configuration lists them.
    pub const ALL: [Self; 9] = [
        Self::Function,
        Self::Impl,
        Self::Trait,
        Self::Closure,
        Self::MatchArm,
        Self::IfChain,
        Self::Block,
        Self::Statements,
        Self::TypeShape,
    ];

    /// A short lowercase name for tables.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Impl => "impl",
            Self::Trait => "trait",
            Self::Closure => "closure",
            Self::MatchArm => "match arm",
            Self::IfChain => "if chain",
            Self::Block => "block",
            Self::Statements => "statements",
            Self::TypeShape => "type",
        }
    }
}

/// How alike the copies in a group are, in the standard clone taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneKind {
    /// Type 1: identical apart from whitespace, comments, and attributes.
    Exact,
    /// Type 2: identical structure, with names or literals changed.
    Renamed,
    /// Type 3: similar structure, with statements added, removed, reordered,
    /// or changed.
    NearMiss,
}

impl CloneKind {
    /// A short lowercase name for tables.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Renamed => "renamed",
            Self::NearMiss => "near-miss",
        }
    }
}

/// Which detection layer contributed a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detector {
    /// Maximal repeated token runs from a suffix array (the `CCFinder` method).
    SuffixArray,
    /// Equal identifier-blind subtree hashes.
    SubtreeHash,
    /// Equal subtree hashes only once independent statements were reordered.
    Reordered,
    /// `MinHash` / LSH candidates confirmed by node-kind Dice similarity.
    MinHash,
    /// Candidates confirmed by Zhang–Shasha tree edit distance.
    EditDistance,
}

/// One copy in a clone group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneInstance {
    /// File, relative to the analysis root.
    pub file: String,
    /// First line of the copy, one-based.
    pub start_line: usize,
    /// Last line of the copy, one-based and inclusive.
    pub end_line: usize,
    /// The item the copy sits in or is, such as `Parser::advance`.
    pub item: Option<String>,
    /// Whether a suggestion may ask for this copy to change.
    pub editable: bool,
    /// Whether the copy is test code.
    pub is_test: bool,
}

impl CloneInstance {
    /// Lines the copy spans.
    #[must_use]
    pub const fn lines(&self) -> usize {
        self.end_line
            .saturating_sub(self.start_line)
            .saturating_add(1)
    }
}

/// What kind of shared code a group suggests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SketchKind {
    /// One function, with a parameter for each difference.
    Function,
    /// One generic function, impl, or trait default: only types differ.
    Generic,
    /// One `macro_rules!`: the copies differ in names a function cannot take.
    Macro,
    /// A loop over a table: adjacent copies differ only in values.
    Loop,
    /// A helper that calls itself: one copy sits inside another.
    Recursive,
    /// One type definition, or a generic one, shared by every copy.
    SharedType,
}

impl SketchKind {
    /// A short lowercase name for tables.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Generic => "generic",
            Self::Macro => "macro",
            Self::Loop => "loop",
            Self::Recursive => "recursive",
            Self::SharedType => "shared type",
        }
    }
}

/// What a parameter of the suggested helper stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    /// A literal value.
    Literal,
    /// A variable, field, or function name.
    Identifier,
    /// A type.
    Type,
    /// An expression.
    Expression,
    /// Statements present in some copies and not others, or different ones.
    Statements,
}

/// One place where the copies differ, which the shared code would take as a
/// parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parameter {
    /// The parameter's name in the sketch: `p0`, `p1`, … or `T0`, `T1`, …
    pub name: String,
    /// What it stands for.
    pub kind: ParameterKind,
    /// Where it first appears in the first copy.
    pub line: usize,
    /// Its value in each copy, in instance order. Empty when a copy has
    /// nothing there.
    pub values: Vec<String>,
}

/// A suggested replacement for a group of copies.
///
/// A sketch, not a patch: the signature is assembled from the differences
/// between the copies and the kinds of their values, and is meant to tell a
/// reader what shape the shared code would take — not to compile unedited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sketch {
    /// What kind of shared code fits.
    pub kind: SketchKind,
    /// The suggested signature or header.
    pub signature: String,
    /// One sentence on what to do.
    pub summary: String,
    /// The differences that become parameters.
    pub parameters: Vec<Parameter>,
}

/// A set of code fragments that are copies of each other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloneGroup {
    /// Stable identifier: the group's structural hash, in hex.
    pub id: String,
    /// How alike the copies are.
    pub kind: CloneKind,
    /// The granularity they were found at.
    pub fragment: FragmentKind,
    /// Which detectors found them.
    pub detectors: Vec<Detector>,
    /// The lowest pairwise similarity in the group, from `0.0` to `1.0`.
    pub similarity: f64,
    /// Normalized tokens in the first copy.
    pub tokens: usize,
    /// Lines in the first copy.
    pub lines: usize,
    /// Estimated lines removed by replacing every copy with a call: each copy
    /// but one goes, and each copy leaves one line behind.
    pub lines_saved: usize,
    /// Ranking score; meaningful only as an order.
    pub score: f64,
    /// Whether every copy is test code.
    pub in_tests: bool,
    /// Whether every copy is editable.
    pub editable: bool,
    /// Whether one copy sits inside another.
    pub recursive: bool,
    /// The copies, in file and line order.
    pub instances: Vec<CloneInstance>,
    /// What to replace them with.
    pub sketch: Sketch,
}

/// One entry of the symbol index: an item reduced to the parts that matter
/// for finding reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolRecord {
    /// The grammar's kind: `function_item`, `struct_item`, …
    pub kind: String,
    /// The item's own name.
    pub name: String,
    /// `Type::method` inside an impl, the bare name elsewhere.
    pub qualified_name: String,
    /// File, relative to the analysis root.
    pub file: String,
    /// First line, one-based.
    pub start_line: usize,
    /// Last line, one-based and inclusive.
    pub end_line: usize,
    /// Whether the item may be changed.
    pub editable: bool,
    /// Whether the item is test code.
    pub is_test: bool,
    /// The identifier-blind structural hash, in hex. Equal hashes mean the
    /// items have the same shape.
    pub shape: String,
    /// Normalized tokens in the item.
    pub tokens: usize,
    /// Parameter types, for a function.
    pub parameters: Vec<String>,
    /// Return type, for a function that declares one.
    pub return_type: Option<String>,
    /// Every type name the item mentions, sorted and deduplicated.
    pub types: Vec<String>,
    /// Every value name the item mentions, sorted and deduplicated.
    pub identifiers: Vec<String>,
}
