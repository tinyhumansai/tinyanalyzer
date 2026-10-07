//! What one file says about the module tree, and what resolving it found.

use std::collections::BTreeSet;

/// Everything one parsed Rust file contributes to its crate's module tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileDeclarations {
    /// Every out-of-line `mod name;` the file declares, inline nesting included.
    pub(crate) modules: Vec<ModuleDeclaration>,
    /// Targets of `include!`, `include_str!`, and `include_bytes!` given as a
    /// string literal, relative to the file's own directory.
    pub(crate) includes: Vec<IncludedFile>,
    /// Whether some `include!` names its file with anything but a literal.
    ///
    /// `include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/x.rs"))` can pull
    /// any file of the crate in, so a crate holding one is never searched for
    /// orphans.
    pub(crate) unresolved_include: bool,
    /// Identifiers appearing inside item-position macro invocations.
    ///
    /// A macro can expand to `mod name;`, and its expansion is not visible
    /// here. A file whose module name appears in this set anywhere in its
    /// crate is never reported as an orphan.
    pub(crate) macro_identifiers: BTreeSet<String>,
    /// Top-level items in the file, `use` declarations included.
    ///
    /// A file with none is never reported: there is nothing in it to delete.
    pub(crate) items: usize,
}

/// One `mod name;` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleDeclaration {
    /// The module's name.
    pub(crate) name: String,
    /// The directory components contributed by enclosing inline modules,
    /// outermost first: each inline module's `#[path]` if it has one,
    /// otherwise its name.
    pub(crate) inline_path: Vec<String>,
    /// `#[path = "..."]` values, unconditional ones first.
    pub(crate) paths: Vec<String>,
    /// Whether one of `paths` applies unconditionally.
    ///
    /// When every path sits inside a `cfg_attr`, the default location is also
    /// a possible target, depending on configuration.
    pub(crate) path_is_unconditional: bool,
    /// Whether the declaration sits under `#[cfg(test)]`, on itself, on an
    /// enclosing inline module, or on the whole file.
    pub(crate) is_test: bool,
}

/// A file named by an `include*!` macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IncludedFile {
    /// The literal path, relative to the including file's directory.
    pub(crate) path: String,
    /// Whether the file is pasted in as Rust (`include!`) rather than as data.
    ///
    /// Module declarations inside an `include!`d file belong to the including
    /// module, so they are followed; a data include is only marked reachable.
    pub(crate) is_source: bool,
}

/// One file of a crate's `src/` directory that no crate root reaches.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct OrphanFile {
    /// The file, relative to the analysis root.
    pub(crate) path: String,
    /// The package whose `src/` it sits in.
    pub(crate) crate_name: String,
}

/// What resolving every crate's module tree found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModuleTree {
    /// Files under a crate's `src/` that no `mod` declaration reaches.
    pub(crate) orphans: Vec<OrphanFile>,
    /// Files reached only through `#[cfg(test)]` declarations or test targets.
    pub(crate) test_only: BTreeSet<String>,
}
