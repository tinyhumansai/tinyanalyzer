//! Reading the module declarations out of one parsed file.
//!
//! Run on the syntax tree [`crate::rust_source`] already built, so the module
//! tree costs no second parse. Everything here is syntactic: which `mod name;`
//! items exist, under which inline modules and `#[path]` attributes, whether a
//! `#[cfg(test)]` gates them, and which files an `include*!` macro names.

use super::types::{FileDeclarations, IncludedFile, ModuleDeclaration};
use crate::rust_source::has_cfg_test;
use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};

/// Collects what `file` contributes to its crate's module tree.
pub(crate) fn collect(file: &syn::File) -> FileDeclarations {
    let mut collector = Collector {
        declarations: FileDeclarations {
            items: file.items.len(),
            ..FileDeclarations::default()
        },
        inline_path: Vec::new(),
        in_test: has_cfg_test(&file.attrs),
    };
    collector.visit_file(file);
    collector.declarations
}

/// The walker behind [`collect`].
struct Collector {
    declarations: FileDeclarations,
    /// Directory components of the inline modules enclosing the position.
    inline_path: Vec<String>,
    /// Whether the position is under `#[cfg(test)]`.
    in_test: bool,
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        let (paths, path_is_unconditional) = path_attributes(&node.attrs);
        let is_test = self.in_test || has_cfg_test(&node.attrs);

        let Some((_, items)) = &node.content else {
            self.declarations.modules.push(ModuleDeclaration {
                name: node.ident.to_string(),
                inline_path: self.inline_path.clone(),
                paths,
                path_is_unconditional,
                is_test,
            });
            return;
        };

        // An inline module contributes a directory to the declarations inside
        // it: its own `#[path]` when it has one, otherwise its name.
        let component = paths
            .into_iter()
            .next()
            .unwrap_or_else(|| node.ident.to_string());
        let was_in_test = self.in_test;
        self.inline_path.push(component);
        self.in_test = is_test;
        for item in items {
            self.visit_item(item);
        }
        self.in_test = was_in_test;
        self.inline_path.pop();
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        // `cfg_if!` and its relatives hide real declarations in their bodies:
        // `mod name;` sequences are taken as declarations, and every other
        // identifier is remembered as a name some expansion might declare.
        let is_test = self.in_test || has_cfg_test(&node.attrs);
        let mut identifiers = Vec::new();
        scan_macro_tokens(node.mac.tokens.clone(), &mut identifiers, &mut |name| {
            self.declarations.modules.push(ModuleDeclaration {
                name,
                inline_path: self.inline_path.clone(),
                paths: Vec::new(),
                path_is_unconditional: false,
                is_test,
            });
        });
        self.declarations.macro_identifiers.extend(identifiers);

        visit::visit_item_macro(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let Some(name) = node.path.segments.last().map(|segment| segment.ident.to_string()) else {
            return;
        };
        let is_source = name == "include";
        if !is_source && name != "include_str" && name != "include_bytes" {
            return;
        }

        match syn::parse2::<syn::LitStr>(node.tokens.clone()) {
            Ok(literal) => self.declarations.includes.push(IncludedFile {
                path: literal.value(),
                is_source,
            }),
            // A data include the analysis cannot resolve only hides a data
            // file; a source include could hide a module.
            Err(_) if is_source => self.declarations.unresolved_include = true,
            Err(_) => {}
        }
    }
}

/// The `#[path]` values on a module, and whether one is unconditional.
///
/// `#[cfg_attr(condition, path = "x.rs")]` contributes a path too, but only
/// conditionally: with the condition off, the default location applies.
fn path_attributes(attrs: &[syn::Attribute]) -> (Vec<String>, bool) {
    let mut unconditional = Vec::new();
    let mut conditional = Vec::new();

    for attr in attrs {
        if attr.path().is_ident("path") {
            if let syn::Meta::NameValue(pair) = &attr.meta
                && let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(literal),
                    ..
                }) = &pair.value
            {
                unconditional.push(literal.value());
            }
        } else if attr.path().is_ident("cfg_attr")
            && let syn::Meta::List(list) = &attr.meta
        {
            conditional.extend(path_assignments(list.tokens.clone()));
        }
    }

    let is_unconditional = !unconditional.is_empty();
    unconditional.extend(conditional);
    (unconditional, is_unconditional)
}

/// Every `path = "..."` sequence in a token stream, at any depth.
fn path_assignments(tokens: TokenStream) -> Vec<String> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut found = Vec::new();

    for (index, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Ident(ident) if ident == "path" => {
                if let (Some(TokenTree::Punct(eq)), Some(TokenTree::Literal(value))) =
                    (trees.get(index + 1), trees.get(index + 2))
                    && eq.as_char() == '='
                    && let Some(text) = string_literal(value)
                {
                    found.push(text);
                }
            }
            TokenTree::Group(group) => found.extend(path_assignments(group.stream())),
            _ => {}
        }
    }

    found
}

/// The value of a string literal token, or `None` for any other literal.
fn string_literal(literal: &proc_macro2::Literal) -> Option<String> {
    syn::parse2::<syn::LitStr>(TokenTree::Literal(literal.clone()).into())
        .ok()
        .map(|literal| literal.value())
}

/// Walks a macro body, reporting `mod name;` sequences through `declare` and
/// collecting every other identifier into `identifiers`.
fn scan_macro_tokens(
    tokens: TokenStream,
    identifiers: &mut Vec<String>,
    declare: &mut impl FnMut(String),
) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();

    for (index, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Ident(ident) if ident == "mod" => {
                if let (Some(TokenTree::Ident(name)), Some(TokenTree::Punct(semi))) =
                    (trees.get(index + 1), trees.get(index + 2))
                    && semi.as_char() == ';'
                {
                    declare(name.to_string());
                }
            }
            TokenTree::Ident(ident) => identifiers.push(ident.to_string()),
            TokenTree::Group(group) => scan_macro_tokens(group.stream(), identifiers, declare),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
}
