//! What parsing one Rust file tells you about it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Everything the parser learned about a single `.rs` file.
///
/// Produced by [`analyze`](super::analyze). A file that fails to parse — a
/// syntax error, or a dialect this `syn` does not know — produces no
/// `RustFile` at all rather than a partial one, so nothing downstream has to
/// wonder whether a zero means "none" or "unknown".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RustFile {
    /// How many of each kind of item the file defines.
    pub items: ItemCounts,
    /// Every function and method, in source order.
    pub functions: Vec<Function>,
    /// Every named item the file defines, for dead-code analysis.
    pub definitions: Vec<Definition>,
    /// Crate names this file names in a `use` or a path.
    ///
    /// The first segment of every path, minus `self`, `super`, and `crate`.
    /// This is what unused-dependency detection is built on.
    pub referenced_crates: Vec<String>,
    /// How many times each identifier appears anywhere in the file.
    ///
    /// Counted from the token stream rather than the AST, so an identifier used
    /// only inside a macro invocation still counts. Dead-code analysis sums
    /// these across the workspace and asks which definitions never show up.
    pub identifier_uses: BTreeMap<String, usize>,
    /// Signals that bear on runtime cost.
    pub performance: PerformanceSignals,
    /// Items declared `pub`, at any level.
    pub public_items: usize,
    /// `unsafe` blocks and `unsafe` functions.
    ///
    /// Always zero in a workspace that forbids `unsafe_code`, which is worth
    /// showing rather than hiding: it is the cheapest possible confirmation
    /// that the lint is actually on.
    pub unsafe_blocks: usize,
    /// Whether the file's contents are entirely behind `#[cfg(test)]`.
    pub is_test_module: bool,
    /// `TODO`, `FIXME`, `HACK`, and `XXX` markers in comments.
    pub todo_markers: usize,
    /// The deepest block nesting reached anywhere in the file.
    pub max_nesting: usize,
}

/// How many of each kind of item a file defines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemCounts {
    /// Free functions and methods.
    pub functions: usize,
    /// `struct` definitions.
    pub structs: usize,
    /// `enum` definitions.
    pub enums: usize,
    /// `trait` definitions.
    pub traits: usize,
    /// `impl` blocks, inherent and trait.
    pub impls: usize,
    /// `mod` declarations, inline and out-of-line.
    pub modules: usize,
    /// `macro_rules!` definitions.
    pub macros: usize,
    /// `const` items.
    pub consts: usize,
    /// `static` items.
    pub statics: usize,
    /// `type` aliases.
    pub type_aliases: usize,
    /// `use` declarations.
    pub imports: usize,
}

impl ItemCounts {
    /// Every item, of every kind.
    #[must_use]
    pub const fn total(self) -> usize {
        self.functions
            + self.structs
            + self.enums
            + self.traits
            + self.impls
            + self.modules
            + self.macros
            + self.consts
            + self.statics
            + self.type_aliases
    }

    /// Adds another file's counts into this one.
    pub fn add(&mut self, other: Self) {
        self.functions = self.functions.saturating_add(other.functions);
        self.structs = self.structs.saturating_add(other.structs);
        self.enums = self.enums.saturating_add(other.enums);
        self.traits = self.traits.saturating_add(other.traits);
        self.impls = self.impls.saturating_add(other.impls);
        self.modules = self.modules.saturating_add(other.modules);
        self.macros = self.macros.saturating_add(other.macros);
        self.consts = self.consts.saturating_add(other.consts);
        self.statics = self.statics.saturating_add(other.statics);
        self.type_aliases = self.type_aliases.saturating_add(other.type_aliases);
        self.imports = self.imports.saturating_add(other.imports);
    }
}

/// One function or method.
//
// The `is_*` flags are five independent facts about one function, each of them a
// column an operator sorts and filters the dashboard by. Folding them into a
// bitflag or an enum would cost the report its readable serialized form without
// removing a single piece of state.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    /// The function's own name, without any `impl` qualification.
    pub name: String,
    /// Qualified name, `Type::method` for a method and the bare name otherwise.
    ///
    /// This is what a reader needs to find the function again; two `new`s in
    /// one file are otherwise indistinguishable in a ranked table.
    pub qualified_name: String,
    /// One-based line the signature starts on.
    pub start_line: usize,
    /// One-based line the body ends on.
    pub end_line: usize,
    /// Cyclomatic complexity: one, plus one per branch.
    ///
    /// Counted over `if`, `match` arms, `while`, `for`, `loop`, `&&`, `||`, and
    /// `?`. It is an approximation of how many paths run through the function,
    /// and its value is comparative — the number itself means little, the
    /// ranking means a lot.
    pub complexity: u32,
    /// Deepest block nesting inside the body.
    pub max_nesting: usize,
    /// Number of declared parameters, `self` included.
    pub parameters: usize,
    /// Whether the function is `pub`.
    pub is_public: bool,
    /// Whether the function is `async`.
    pub is_async: bool,
    /// Whether the function is `unsafe`.
    pub is_unsafe: bool,
    /// Whether the function has type, lifetime, or const generics.
    ///
    /// A generic function is compiled once per instantiation, so a large
    /// generic body is a build-time and binary-size cost as well as a
    /// readability one.
    pub is_generic: bool,
    /// Whether the function carries `#[test]` or sits under `#[cfg(test)]`.
    pub is_test: bool,
}

impl Function {
    /// Lines the function body spans, inclusive of the signature.
    #[must_use]
    pub const fn lines(&self) -> usize {
        // `end_line` is produced by the parser from the same span as
        // `start_line`, so it is never the smaller of the two; saturating
        // subtraction states that rather than relying on it.
        self.end_line.saturating_sub(self.start_line).saturating_add(1)
    }
}

/// A named item that dead-code analysis can ask about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// The item's name.
    pub name: String,
    /// What kind of item it is.
    pub kind: DefinitionKind,
    /// One-based line the item starts on.
    pub line: usize,
    /// Whether the item is `pub`.
    pub is_public: bool,
    /// Whether the item is test-only.
    pub is_test: bool,
    /// Whether the item is reachable from outside the crate regardless of use.
    ///
    /// A trait method implementation, an ABI export, or anything carrying an
    /// attribute that makes a name meaningful to something other than Rust.
    /// These are never reported as dead, because the detector cannot see the
    /// caller.
    pub is_externally_reachable: bool,
}

/// The kind of a [`Definition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DefinitionKind {
    /// A free function or an inherent method.
    Function,
    /// A `struct`.
    Struct,
    /// An `enum`.
    Enum,
    /// A `trait`.
    Trait,
    /// A `const`.
    Const,
    /// A `static`.
    Static,
    /// A `type` alias.
    TypeAlias,
    /// A `macro_rules!` definition.
    Macro,
    /// A `mod` declaration.
    Module,
}

impl DefinitionKind {
    /// The word used for this kind on the dashboard.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Const => "const",
            Self::Static => "static",
            Self::TypeAlias => "type alias",
            Self::Macro => "macro",
            Self::Module => "module",
        }
    }
}

/// Counts of constructs that bear on runtime cost.
///
/// None of these is a defect on its own. They are the raw material the rules in
/// [`crate::findings`] turn into advice, and they are reported unaggregated so
/// a reader can disagree with a rule and still see what it saw.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceSignals {
    /// `.clone()` calls.
    pub clones: usize,
    /// Calls that allocate a fresh owned value: `to_string`, `to_owned`,
    /// `to_vec`, and the `format!` macro.
    pub allocating_conversions: usize,
    /// `.collect()` calls.
    pub collects: usize,
    /// `.unwrap()` and `.expect()` calls.
    ///
    /// A panic path, and in a library a correctness problem rather than a
    /// performance one — but it is found by the same pass and belongs in the
    /// same table.
    pub unwraps: usize,
    /// `dyn Trait` occurrences, each one a virtual call site.
    pub dyn_dispatch: usize,
    /// Functions with generic parameters, each one a monomorphization.
    pub generic_functions: usize,
    /// `async` functions.
    pub async_functions: usize,
    /// Loops directly containing another loop.
    ///
    /// The cheapest available signal for quadratic behavior.
    pub nested_loops: usize,
    /// Allocating calls that occur inside a loop body.
    ///
    /// An allocation per iteration is the single most common avoidable cost in
    /// Rust code that is otherwise well written.
    pub allocations_in_loops: usize,
}

impl PerformanceSignals {
    /// Adds another file's signals into these.
    pub fn add(&mut self, other: Self) {
        self.clones = self.clones.saturating_add(other.clones);
        self.allocating_conversions = self
            .allocating_conversions
            .saturating_add(other.allocating_conversions);
        self.collects = self.collects.saturating_add(other.collects);
        self.unwraps = self.unwraps.saturating_add(other.unwraps);
        self.dyn_dispatch = self.dyn_dispatch.saturating_add(other.dyn_dispatch);
        self.generic_functions = self
            .generic_functions
            .saturating_add(other.generic_functions);
        self.async_functions = self.async_functions.saturating_add(other.async_functions);
        self.nested_loops = self.nested_loops.saturating_add(other.nested_loops);
        self.allocations_in_loops = self
            .allocations_in_loops
            .saturating_add(other.allocations_in_loops);
    }
}
