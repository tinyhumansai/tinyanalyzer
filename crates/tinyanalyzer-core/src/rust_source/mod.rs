//! Parsing Rust source into structure, cost signals, and definitions.
//!
//! This is the only module that understands Rust as a language rather than as
//! text. Everything it reports comes from a real parse: item counts, function
//! lengths, cyclomatic complexity, nesting, and the definition list that
//! dead-code analysis is built on. A regular expression can approximate some of
//! that; it cannot tell a `fn` in a doc comment from a `fn` in the program, and
//! a tool whose job is to point at the worst file in a repository has to be
//! right about which file that is.
//!
//! Two things are read from the raw text rather than the syntax tree, because
//! the parser deliberately discards them:
//!
//! - **`TODO` markers**, which live in comments.
//! - **Identifier occurrences**, counted from the token stream so that a name
//!   used only inside a macro invocation still counts as used. The AST would
//!   show the macro call as an opaque token blob and report the name dead.
//!
//! A file that does not parse yields [`ParseFailure`] rather than a partial
//! result, so no number downstream has to be read as "or maybe the parser gave
//! up here".
//!
//! # Example
//!
//! ```
//! use tinyanalyzer_core::rust_source::analyze;
//!
//! let parsed = analyze("pub fn add(a: u8, b: u8) -> u8 { if a > b { a } else { b } }")?;
//!
//! assert_eq!(parsed.items.functions, 1);
//! assert_eq!(parsed.public_items, 1);
//! assert_eq!(parsed.functions[0].qualified_name, "add");
//! assert_eq!(parsed.functions[0].parameters, 2);
//! assert_eq!(parsed.functions[0].complexity, 2);
//! # Ok::<(), tinyanalyzer_core::rust_source::ParseFailure>(())
//! ```

mod types;

pub use types::{
    Definition, DefinitionKind, Function, ItemCounts, PerformanceSignals, RustFile,
};

use proc_macro2::TokenTree;
use std::collections::BTreeMap;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// Why a file could not be parsed.
///
/// Carries the line so an operator can go straight to it. A parse failure is
/// not an analysis failure — the file still appears in the report with its line
/// counts — so this is deliberately not a [`crate::Error`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseFailure {
    /// One-based line the parser gave up on.
    pub line: usize,
    /// What the parser objected to.
    pub message: String,
}

/// Comment markers counted as unfinished work.
const TODO_MARKERS: [&str; 4] = ["TODO", "FIXME", "HACK", "XXX"];

/// Method names that allocate a fresh owned value.
const ALLOCATING_METHODS: [&str; 5] = ["to_string", "to_owned", "to_vec", "into_owned", "into_vec"];

/// Method names that can panic.
const PANICKING_METHODS: [&str; 2] = ["unwrap", "expect"];

/// Parses one Rust file and measures it.
///
/// # Errors
///
/// Returns [`ParseFailure`] if `text` is not valid Rust.
pub fn analyze(text: &str) -> std::result::Result<RustFile, ParseFailure> {
    let parsed = syn::parse_file(text).map_err(|error| ParseFailure {
        line: error.span().start().line,
        message: error.to_string(),
    })?;

    let mut visitor = FileVisitor::default();
    visitor.file.is_test_module = has_cfg_test(&parsed.attrs);
    visitor.in_test = visitor.file.is_test_module;
    visitor.visit_file(&parsed);

    let mut file = visitor.file;
    file.todo_markers = count_todo_markers(text);
    file.identifier_uses = count_identifiers(text);
    file.referenced_crates.sort_unstable();
    file.referenced_crates.dedup();

    // A file whose every top-level item is test-gated is a test file even
    // without an inner `#![cfg(test)]`, which is the shape `tests/` modules
    // take when they are split across several files.
    if !file.is_test_module && !file.definitions.is_empty() {
        file.is_test_module = file.definitions.iter().all(|item| item.is_test);
    }

    Ok(file)
}

/// Counts unfinished-work markers in `text`.
///
/// Read from the raw text because the parser discards comments, and a `TODO`
/// almost always lives in one.
fn count_todo_markers(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            let is_comment = trimmed.starts_with("//") || trimmed.starts_with('*');
            is_comment && TODO_MARKERS.iter().any(|marker| line.contains(marker))
        })
        .count()
}

/// Counts every identifier occurrence in `text`.
///
/// Lexing rather than parsing is the point: the token stream keeps the contents
/// of macro invocations, which the syntax tree stores as an opaque blob. A
/// helper called only from inside a `macro_rules!` body would otherwise look
/// unreferenced.
fn count_identifiers(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();

    let Ok(stream) = text.parse::<proc_macro2::TokenStream>() else {
        return counts;
    };

    let mut pending = vec![stream];
    while let Some(stream) = pending.pop() {
        for tree in stream {
            match tree {
                TokenTree::Ident(ident) => {
                    *counts.entry(ident.to_string()).or_insert(0) += 1;
                }
                TokenTree::Group(group) => pending.push(group.stream()),
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
        }
    }

    counts
}

/// Whether an attribute list contains `#[cfg(test)]`.
fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }

        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

/// Whether an attribute list marks a test.
fn is_test_attribute(attrs: &[syn::Attribute]) -> bool {
    has_cfg_test(attrs)
        || attrs.iter().any(|attr| {
            let path = attr.path();
            path.is_ident("test") || path.is_ident("bench")
        })
}

/// Whether an attribute list makes the item's name meaningful outside Rust.
///
/// An ABI export, a registered entry point, or anything a macro will look up by
/// name has no visible caller, so reporting it as dead code would be wrong
/// every time.
fn is_externally_reachable(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        path.is_ident("no_mangle")
            || path.is_ident("export_name")
            || path.is_ident("used")
            || path.is_ident("proc_macro")
            || path.is_ident("proc_macro_derive")
            || path.is_ident("proc_macro_attribute")
            || path.is_ident("macro_export")
            || path.is_ident("ctor")
    })
}

/// Whether a visibility is `pub`.
const fn is_public(visibility: &syn::Visibility) -> bool {
    matches!(visibility, syn::Visibility::Public(_))
}

/// The walker that fills a [`RustFile`].
#[derive(Default)]
struct FileVisitor {
    file: RustFile,
    /// Names of the types whose `impl` blocks are currently open.
    type_stack: Vec<String>,
    /// Whether the current position is inside test-only code.
    in_test: bool,
    /// How many loops enclose the current position.
    loop_depth: usize,
    /// How many blocks enclose the current position.
    nesting: usize,
}

impl FileVisitor {
    /// Records a definition at `span`.
    fn define(
        &mut self,
        name: String,
        kind: DefinitionKind,
        span: proc_macro2::Span,
        public: bool,
        externally_reachable: bool,
    ) {
        if public {
            self.file.public_items = self.file.public_items.saturating_add(1);
        }

        self.file.definitions.push(Definition {
            name,
            kind,
            line: span.start().line,
            is_public: public,
            is_test: self.in_test,
            is_externally_reachable: externally_reachable,
        });
    }

    /// Records one function, from whichever of the three syntactic forms.
    fn record_function(
        &mut self,
        signature: &syn::Signature,
        attrs: &[syn::Attribute],
        block: &syn::Block,
        public: bool,
        define_as_item: bool,
    ) {
        let name = signature.ident.to_string();
        let qualified_name = match self.type_stack.last() {
            Some(owner) => format!("{owner}::{name}"),
            None => name.clone(),
        };

        let is_test = self.in_test || is_test_attribute(attrs);
        let is_generic = !signature.generics.params.is_empty();
        let span = signature.span();

        let mut complexity = ComplexityVisitor::default();
        complexity.visit_block(block);

        self.file.items.functions = self.file.items.functions.saturating_add(1);
        if is_generic {
            self.file.performance.generic_functions =
                self.file.performance.generic_functions.saturating_add(1);
        }
        if signature.asyncness.is_some() {
            self.file.performance.async_functions =
                self.file.performance.async_functions.saturating_add(1);
        }
        if signature.unsafety.is_some() {
            self.file.unsafe_blocks = self.file.unsafe_blocks.saturating_add(1);
        }

        self.file.functions.push(Function {
            name: name.clone(),
            qualified_name,
            start_line: span.start().line,
            end_line: block.span().end().line,
            complexity: complexity.score,
            max_nesting: complexity.max_nesting,
            parameters: signature.inputs.len(),
            is_public: public,
            is_async: signature.asyncness.is_some(),
            is_unsafe: signature.unsafety.is_some(),
            is_generic,
            is_test,
        });

        // Trait and inherent methods are reachable through their trait or their
        // type; only free functions are candidates for being unreferenced.
        if define_as_item {
            let externally_reachable = is_externally_reachable(attrs) || is_test;
            self.define(name, DefinitionKind::Function, span, public, externally_reachable);
        }

        let was_in_test = self.in_test;
        self.in_test = is_test;
        // The signature is walked as well as the body: parameter and return
        // types are where `dyn Trait` and cross-crate paths live, and a walk
        // that only descended into bodies would miss every one of them.
        self.visit_signature(signature);
        self.visit_block(block);
        self.in_test = was_in_test;
    }

    /// Walks an item list with `in_test` forced on for its duration.
    fn with_test_scope<T>(&mut self, is_test: bool, walk: impl FnOnce(&mut Self) -> T) -> T {
        let previous = self.in_test;
        self.in_test = previous || is_test;
        let result = walk(self);
        self.in_test = previous;
        result
    }

    /// Records an allocating call, noting whether it happened inside a loop.
    fn record_allocation(&mut self) {
        self.file.performance.allocating_conversions = self
            .file
            .performance
            .allocating_conversions
            .saturating_add(1);

        if self.loop_depth > 0 {
            self.file.performance.allocations_in_loops =
                self.file.performance.allocations_in_loops.saturating_add(1);
        }
    }

    /// Walks a loop body, tracking depth so nesting can be detected.
    fn in_loop(&mut self, walk: impl FnOnce(&mut Self)) {
        if self.loop_depth > 0 {
            self.file.performance.nested_loops =
                self.file.performance.nested_loops.saturating_add(1);
        }

        self.loop_depth = self.loop_depth.saturating_add(1);
        walk(self);
        self.loop_depth = self.loop_depth.saturating_sub(1);
    }
}

impl<'ast> Visit<'ast> for FileVisitor {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let public = is_public(&node.vis);
        self.record_function(&node.sig, &node.attrs, &node.block, public, true);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let public = is_public(&node.vis);
        self.record_function(&node.sig, &node.attrs, &node.block, public, false);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        match &node.default {
            Some(block) => self.record_function(&node.sig, &node.attrs, block, true, false),
            None => {
                self.file.items.functions = self.file.items.functions.saturating_add(1);
                visit::visit_trait_item_fn(self, node);
            }
        }
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.file.items.structs = self.file.items.structs.saturating_add(1);
        let is_test = is_test_attribute(&node.attrs);
        self.with_test_scope(is_test, |visitor| {
            visitor.define(
                node.ident.to_string(),
                DefinitionKind::Struct,
                node.ident.span(),
                is_public(&node.vis),
                is_externally_reachable(&node.attrs),
            );
            visit::visit_item_struct(visitor, node);
        });
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.file.items.enums = self.file.items.enums.saturating_add(1);
        let is_test = is_test_attribute(&node.attrs);
        self.with_test_scope(is_test, |visitor| {
            visitor.define(
                node.ident.to_string(),
                DefinitionKind::Enum,
                node.ident.span(),
                is_public(&node.vis),
                is_externally_reachable(&node.attrs),
            );
            visit::visit_item_enum(visitor, node);
        });
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.file.items.traits = self.file.items.traits.saturating_add(1);
        let is_test = is_test_attribute(&node.attrs);
        self.with_test_scope(is_test, |visitor| {
            visitor.define(
                node.ident.to_string(),
                DefinitionKind::Trait,
                node.ident.span(),
                is_public(&node.vis),
                is_externally_reachable(&node.attrs),
            );
            visit::visit_item_trait(visitor, node);
        });
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        self.file.items.impls = self.file.items.impls.saturating_add(1);

        let owner = type_name(&node.self_ty).unwrap_or_else(|| "impl".to_owned());
        let is_test = is_test_attribute(&node.attrs);

        self.type_stack.push(owner);
        self.with_test_scope(is_test, |visitor| visit::visit_item_impl(visitor, node));
        self.type_stack.pop();
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.file.items.modules = self.file.items.modules.saturating_add(1);
        let is_test = is_test_attribute(&node.attrs);

        self.with_test_scope(is_test, |visitor| {
            visitor.define(
                node.ident.to_string(),
                DefinitionKind::Module,
                node.ident.span(),
                is_public(&node.vis),
                is_externally_reachable(&node.attrs),
            );
            visit::visit_item_mod(visitor, node);
        });
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.file.items.consts = self.file.items.consts.saturating_add(1);
        let is_test = is_test_attribute(&node.attrs);
        self.with_test_scope(is_test, |visitor| {
            visitor.define(
                node.ident.to_string(),
                DefinitionKind::Const,
                node.ident.span(),
                is_public(&node.vis),
                is_externally_reachable(&node.attrs),
            );
            visit::visit_item_const(visitor, node);
        });
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.file.items.statics = self.file.items.statics.saturating_add(1);
        let is_test = is_test_attribute(&node.attrs);
        self.with_test_scope(is_test, |visitor| {
            visitor.define(
                node.ident.to_string(),
                DefinitionKind::Static,
                node.ident.span(),
                is_public(&node.vis),
                is_externally_reachable(&node.attrs),
            );
            visit::visit_item_static(visitor, node);
        });
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.file.items.type_aliases = self.file.items.type_aliases.saturating_add(1);
        let is_test = is_test_attribute(&node.attrs);
        self.with_test_scope(is_test, |visitor| {
            visitor.define(
                node.ident.to_string(),
                DefinitionKind::TypeAlias,
                node.ident.span(),
                is_public(&node.vis),
                is_externally_reachable(&node.attrs),
            );
            visit::visit_item_type(visitor, node);
        });
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        self.file.items.macros = self.file.items.macros.saturating_add(1);

        if let Some(ident) = &node.ident {
            let is_test = is_test_attribute(&node.attrs);
            self.with_test_scope(is_test, |visitor| {
                visitor.define(
                    ident.to_string(),
                    DefinitionKind::Macro,
                    ident.span(),
                    // A `macro_rules!` has no visibility of its own; being
                    // exported is what makes it public.
                    is_externally_reachable(&node.attrs),
                    is_externally_reachable(&node.attrs),
                );
            });
        }

        visit::visit_item_macro(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        self.file.items.imports = self.file.items.imports.saturating_add(1);

        if let Some(root) = use_tree_root(&node.tree) {
            self.file.referenced_crates.push(root);
        }

        visit::visit_item_use(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        if node.segments.len() > 1
            && let Some(first) = node.segments.first()
        {
            let root = first.ident.to_string();
            if !matches!(root.as_str(), "self" | "super" | "crate" | "Self") {
                self.file.referenced_crates.push(root);
            }
        }

        visit::visit_path(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method = node.method.to_string();

        if method == "clone" {
            self.file.performance.clones = self.file.performance.clones.saturating_add(1);
            if self.loop_depth > 0 {
                self.file.performance.allocations_in_loops =
                    self.file.performance.allocations_in_loops.saturating_add(1);
            }
        } else if ALLOCATING_METHODS.contains(&method.as_str()) {
            self.record_allocation();
        } else if method == "collect" {
            self.file.performance.collects = self.file.performance.collects.saturating_add(1);
            if self.loop_depth > 0 {
                self.file.performance.allocations_in_loops =
                    self.file.performance.allocations_in_loops.saturating_add(1);
            }
        } else if PANICKING_METHODS.contains(&method.as_str()) {
            self.file.performance.unwraps = self.file.performance.unwraps.saturating_add(1);
        }

        visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if let Some(name) = node.path.segments.last()
            && matches!(name.ident.to_string().as_str(), "format" | "vec")
        {
            self.record_allocation();
        }

        visit::visit_macro(self, node);
    }

    fn visit_type_trait_object(&mut self, node: &'ast syn::TypeTraitObject) {
        self.file.performance.dyn_dispatch =
            self.file.performance.dyn_dispatch.saturating_add(1);
        visit::visit_type_trait_object(self, node);
    }

    fn visit_expr_unsafe(&mut self, node: &'ast syn::ExprUnsafe) {
        self.file.unsafe_blocks = self.file.unsafe_blocks.saturating_add(1);
        visit::visit_expr_unsafe(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.in_loop(|visitor| visit::visit_expr_for_loop(visitor, node));
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.in_loop(|visitor| visit::visit_expr_while(visitor, node));
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.in_loop(|visitor| visit::visit_expr_loop(visitor, node));
    }

    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.nesting = self.nesting.saturating_add(1);
        self.file.max_nesting = self.file.max_nesting.max(self.nesting);
        visit::visit_block(self, node);
        self.nesting = self.nesting.saturating_sub(1);
    }
}

/// Renders the name of the type an `impl` block is for.
///
/// Returns `None` for a type with no single leading name — a tuple or a
/// reference — where there is nothing useful to qualify a method with.
fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        syn::Type::Reference(reference) => type_name(&reference.elem),
        _ => None,
    }
}

/// The crate or module a `use` tree starts from.
fn use_tree_root(tree: &syn::UseTree) -> Option<String> {
    let root = match tree {
        syn::UseTree::Path(path) => path.ident.to_string(),
        syn::UseTree::Name(name) => name.ident.to_string(),
        syn::UseTree::Rename(rename) => rename.ident.to_string(),
        syn::UseTree::Glob(_) | syn::UseTree::Group(_) => return None,
    };

    if matches!(root.as_str(), "self" | "super" | "crate") {
        return None;
    }

    Some(root)
}

/// Scores one function body's cyclomatic complexity and nesting.
///
/// Kept separate from [`FileVisitor`] because it is run per function on the
/// body alone: a shared visitor would have to unwind its counters at every
/// function boundary, and that bookkeeping is exactly where an off-by-one hides.
#[derive(Debug)]
struct ComplexityVisitor {
    score: u32,
    nesting: usize,
    max_nesting: usize,
}

impl Default for ComplexityVisitor {
    fn default() -> Self {
        // A function with no branches has exactly one path through it.
        Self {
            score: 1,
            nesting: 0,
            max_nesting: 0,
        }
    }
}

impl ComplexityVisitor {
    fn branch(&mut self) {
        self.score = self.score.saturating_add(1);
    }
}

impl<'ast> Visit<'ast> for ComplexityVisitor {
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        self.branch();
        visit::visit_expr_if(self, node);
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        self.branch();
        visit::visit_arm(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.branch();
        visit::visit_expr_while(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.branch();
        visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.branch();
        visit::visit_expr_loop(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.branch();
        visit::visit_expr_try(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        if matches!(node.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
            self.branch();
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.nesting = self.nesting.saturating_add(1);
        self.max_nesting = self.max_nesting.max(self.nesting);
        visit::visit_block(self, node);
        self.nesting = self.nesting.saturating_sub(1);
    }

    // A nested function is its own unit of complexity; counting its branches
    // against its parent would make a file of small helpers look like one
    // enormous function.
    fn visit_item_fn(&mut self, _node: &'ast syn::ItemFn) {}
}

#[cfg(test)]
mod test;
