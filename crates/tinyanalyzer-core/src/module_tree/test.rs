//! Unit tests for module-tree resolution.
//!
//! Each fixture is a set of in-memory files and manifests: resolution is pure
//! path arithmetic over what the walk produced, and the one filesystem question
//! it asks — does an unwalked path exist? — is answered by the fixture too.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::path::{join, parent, stem};
use super::roots::{Root, roots};
use super::{FileDeclarations, ModuleInput, ModuleTree, PackageManifest, collect, resolve};
use std::collections::BTreeSet;

fn declarations(source: &str) -> FileDeclarations {
    collect(&syn::parse_file(source).expect("the fixture is valid Rust"))
}

/// A package at `directory` with the given manifest and files.
struct Fixture {
    packages: Vec<(String, String, toml::Value)>,
    files: Vec<(String, Option<FileDeclarations>)>,
    on_disk_only: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            packages: Vec::new(),
            files: Vec::new(),
            on_disk_only: Vec::new(),
        }
    }

    fn package(mut self, directory: &str, name: &str, manifest: &str) -> Self {
        self.packages.push((
            directory.to_owned(),
            name.to_owned(),
            toml::from_str(manifest).expect("the fixture manifest is valid TOML"),
        ));
        self
    }

    fn file(mut self, path: &str, source: &str) -> Self {
        self.files
            .push((path.to_owned(), Some(declarations(source))));
        self
    }

    fn unparsed(mut self, path: &str) -> Self {
        self.files.push((path.to_owned(), None));
        self
    }

    fn excluded(mut self, path: &str) -> Self {
        self.on_disk_only.push(path.to_owned());
        self
    }

    fn resolve(&self) -> ModuleTree {
        let packages: Vec<PackageManifest<'_>> = self
            .packages
            .iter()
            .map(|(directory, name, manifest)| PackageManifest {
                directory,
                name,
                manifest,
            })
            .collect();
        let files: Vec<ModuleInput<'_>> = self
            .files
            .iter()
            .map(|(path, declarations)| ModuleInput {
                path,
                declarations: declarations.as_ref(),
            })
            .collect();
        let on_disk: BTreeSet<&str> = self.on_disk_only.iter().map(String::as_str).collect();
        resolve(&packages, &files, &|path| on_disk.contains(path))
    }

    fn orphans(&self) -> Vec<String> {
        self.resolve()
            .orphans
            .into_iter()
            .map(|orphan| orphan.path)
            .collect()
    }
}

const PACKAGE: &str = "[package]\nname = \"demo\"\n";

fn demo() -> Fixture {
    Fixture::new().package(".", "demo", PACKAGE)
}

// ---- path arithmetic ----

#[test]
fn joining_folds_dots_and_climbs_out_of_directories() {
    assert_eq!(join(".", "src/lib.rs").as_deref(), Some("src/lib.rs"));
    assert_eq!(join("src/a", "../b.rs").as_deref(), Some("src/b.rs"));
    assert_eq!(join("src", "./x\\y.rs").as_deref(), Some("src/x/y.rs"));
}

#[test]
fn joining_refuses_absolute_paths_and_paths_above_the_root() {
    assert_eq!(join("src", "/etc/passwd"), None);
    assert_eq!(join("src", "../../x.rs"), None);
}

#[test]
fn parent_and_stem_name_the_directory_and_the_module() {
    assert_eq!(parent("lib.rs"), ".");
    assert_eq!(parent("src/a/b.rs"), "src/a");
    assert_eq!(stem("src/a/b.rs"), "b");
    assert_eq!(stem("README"), "README");
}

// ---- declarations ----

#[test]
fn a_cfg_test_path_declaration_is_read_with_its_path() {
    let found = declarations("#[cfg(test)]\n#[path = \"foo_tests.rs\"]\nmod tests;\n");

    assert_eq!(found.modules.len(), 1);
    let module = &found.modules[0];
    assert_eq!(module.name, "tests");
    assert_eq!(module.paths, ["foo_tests.rs"]);
    assert!(module.path_is_unconditional);
    assert!(module.is_test);
}

#[test]
fn inline_modules_contribute_their_name_or_their_path_as_a_directory() {
    let found = declarations(
        "mod outer { mod inner; #[path = \"elsewhere\"] mod moved { #[cfg(test)] mod deep; } }\n",
    );

    let inner = &found.modules[0];
    assert_eq!(inner.inline_path, ["outer"]);
    assert!(!inner.is_test);
    let deep = &found.modules[1];
    assert_eq!(deep.inline_path, ["outer", "elsewhere"]);
    assert!(deep.is_test);
}

#[test]
fn a_cfg_test_inline_module_makes_its_declarations_test_only() {
    let found = declarations("#[cfg(test)]\nmod tests { mod helpers; }\n");

    assert!(found.modules[0].is_test);
}

#[test]
fn a_file_level_cfg_test_makes_every_declaration_test_only() {
    let found = declarations("#![cfg(test)]\nmod a;\n");

    assert!(found.modules[0].is_test);
}

#[test]
fn a_cfg_attr_path_is_conditional() {
    let found = declarations(
        "#[cfg_attr(feature = \"x\", path = \"x_impl.rs\")]\n#[cfg_attr(all(unix), doc = \"d\")]\nmod imp;\n",
    );

    assert_eq!(found.modules[0].paths, ["x_impl.rs"]);
    assert!(!found.modules[0].path_is_unconditional);
}

#[test]
fn a_mod_inside_an_item_macro_is_a_declaration_and_its_identifiers_are_kept() {
    let found = declarations(
        "cfg_if::cfg_if! { if #[cfg(unix)] { mod unix_impl; } else { mod other; } }\ndeclare!(generated);\n",
    );

    let names: Vec<&str> = found
        .modules
        .iter()
        .map(|module| module.name.as_str())
        .collect();
    assert_eq!(names, ["unix_impl", "other"]);
    assert!(found.macro_identifiers.contains("generated"));
    assert!(found.macro_identifiers.contains("unix"));
}

#[test]
fn includes_are_read_when_literal_and_flagged_when_not() {
    let literal = declarations(
        "include!(\"generated.rs\");\nconst T: &str = include_str!(\"template.rs\");\nfn f() -> &'static [u8] { include_bytes!(concat!(\"a\", \"b\")) }\n",
    );
    assert_eq!(literal.includes.len(), 2);
    assert!(literal.includes[0].is_source);
    assert!(!literal.includes[1].is_source);
    assert!(!literal.unresolved_include);

    let computed = declarations("include!(concat!(env!(\"OUT_DIR\"), \"/x.rs\"));\n");
    assert!(computed.unresolved_include);
}

#[test]
fn other_macros_are_not_includes() {
    let found = declarations("fn f() { println!(\"x\"); }\n");

    assert_eq!(found.includes, []);
    assert_eq!(found.items, 1);
}

// ---- roots ----

#[test]
fn every_conventional_and_explicit_target_is_a_root() {
    let manifest: toml::Value = toml::from_str(
        "[package]\nname = \"demo\"\nbuild = \"tools/build.rs\"\n[lib]\npath = \"lib/root.rs\"\n[[bin]]\nname = \"b\"\npath = \"cli/main.rs\"\n[[test]]\nname = \"t\"\npath = \"../shared/t.rs\"\n[[bench]]\nname = \"x\"\n",
    )
    .unwrap();
    let files = [
        "crate/src/bin/one.rs",
        "crate/src/bin/two/main.rs",
        "crate/src/bin/two/helper.rs",
        "crate/tests/api.rs",
        "crate/examples/demo.rs",
        "crate/benches/speed/main.rs",
        "crate/src/lib.rs",
    ];

    let found = roots("crate", &manifest, &files);

    let expect = |path: &str, is_test: bool| Root {
        path: path.to_owned(),
        is_test,
    };
    for root in [
        expect("crate/build.rs", false),
        expect("crate/tools/build.rs", false),
        expect("crate/lib/root.rs", false),
        expect("crate/cli/main.rs", false),
        expect("shared/t.rs", true),
        expect("crate/src/bin/one.rs", false),
        expect("crate/src/bin/two/main.rs", false),
        expect("crate/tests/api.rs", true),
        expect("crate/examples/demo.rs", false),
        expect("crate/benches/speed/main.rs", true),
    ] {
        assert!(found.contains(&root), "missing {root:?} in {found:?}");
    }
    assert!(!found.iter().any(|root| root.path.ends_with("helper.rs")));
}

#[test]
fn a_file_that_is_both_a_production_and_a_test_root_is_production() {
    let manifest: toml::Value = toml::from_str(
        "[package]\nname = \"demo\"\n[[test]]\nname = \"t\"\npath = \"src/main.rs\"\n",
    )
    .unwrap();

    let found = roots(".", &manifest, &[]);

    let main = found
        .iter()
        .find(|root| root.path == "src/main.rs")
        .expect("main is a root");
    assert!(!main.is_test);
}

// ---- orphans ----

#[test]
fn a_file_no_declaration_reaches_is_an_orphan() {
    let fixture = demo()
        .file("src/lib.rs", "mod used;\n")
        .file("src/used.rs", "pub fn used() {}\n")
        .file("src/left_over.rs", "pub fn stale() {}\n");

    assert_eq!(fixture.orphans(), ["src/left_over.rs"]);
}

#[test]
fn mod_rs_and_named_file_layouts_both_resolve() {
    let fixture = demo()
        .file("src/lib.rs", "mod a;\nmod b;\n")
        .file("src/a/mod.rs", "mod inner;\n")
        .file("src/a/inner.rs", "fn x() {}\n")
        .file("src/b.rs", "mod nested;\n")
        .file("src/b/nested.rs", "fn y() {}\n")
        .file("src/b/stray.rs", "fn z() {}\n");

    assert_eq!(fixture.orphans(), ["src/b/stray.rs"]);
}

#[test]
fn path_attributes_and_inline_nesting_are_followed() {
    let fixture = demo()
        .file(
            "src/main.rs",
            "#[path = \"platform/linux.rs\"]\nmod sys;\nmod outer { mod inner; #[path = \"moved.rs\"] mod renamed; }\n",
        )
        .file("src/platform/linux.rs", "mod detail;\n")
        .file("src/platform/detail.rs", "fn d() {}\n")
        .file("src/outer/inner.rs", "fn i() {}\n")
        .file("src/outer/moved.rs", "fn m() {}\n");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn a_path_inside_an_inline_module_of_a_named_file_resolves_under_its_stem() {
    let fixture = demo()
        .file("src/lib.rs", "mod a;\n")
        .file("src/a.rs", "mod inline { mod leaf; }\n")
        .file("src/a/inline/leaf.rs", "fn l() {}\n");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn both_targets_of_a_conditional_path_are_reachable() {
    let fixture = demo()
        .file(
            "src/lib.rs",
            "#[cfg_attr(windows, path = \"imp_windows.rs\")]\nmod imp;\n",
        )
        .file("src/imp.rs", "fn unix() {}\n")
        .file("src/imp_windows.rs", "fn windows() {}\n");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn files_reached_only_through_cfg_test_are_test_only() {
    let fixture = demo()
        .file(
            "src/lib.rs",
            "mod engine;\n#[cfg(test)]\n#[path = \"lib_checks.rs\"]\nmod checks;\n",
        )
        .file(
            "src/engine.rs",
            "pub fn run() {}\n#[cfg(test)]\n#[path = \"engine_tests.rs\"]\nmod tests;\n",
        )
        .file(
            "src/engine_tests.rs",
            "use super::*;\n#[test]\nfn runs() { run(); }\n",
        )
        .file("src/lib_checks.rs", "mod support;\n")
        .file("src/lib_checks/support.rs", "fn helper() {}\n");

    let tree = fixture.resolve();

    assert_eq!(tree.orphans, []);
    let test_only: Vec<&str> = tree.test_only.iter().map(String::as_str).collect();
    assert_eq!(
        test_only,
        [
            "src/engine_tests.rs",
            "src/lib_checks.rs",
            "src/lib_checks/support.rs"
        ]
    );
}

#[test]
fn a_file_production_also_reaches_is_not_test_only() {
    let fixture = demo()
        .file(
            "src/lib.rs",
            "mod shared;\n#[cfg(test)]\nmod tests { use super::shared; }\n",
        )
        .file("src/shared.rs", "pub fn s() {}\n")
        .file(
            "tests/api.rs",
            "#[path = \"../src/shared.rs\"]\nmod shared;\n",
        );

    assert!(fixture.resolve().test_only.contains("tests/api.rs"));
    assert!(!fixture.resolve().test_only.contains("src/shared.rs"));
}

#[test]
fn a_file_a_test_target_reaches_through_a_path_is_not_an_orphan() {
    let fixture = demo()
        .file("src/lib.rs", "pub fn l() {}\n")
        .file("src/fixtures.rs", "pub fn f() {}\n")
        .file(
            "tests/api.rs",
            "#[path = \"../src/fixtures.rs\"]\nmod fixtures;\n",
        );

    let tree = fixture.resolve();
    assert_eq!(tree.orphans, []);
    assert!(tree.test_only.contains("src/fixtures.rs"));
}

#[test]
fn an_included_source_file_is_reached_and_its_declarations_followed() {
    let fixture = demo()
        .file("src/lib.rs", "include!(\"parts/generated.rs\");\n")
        .file("src/parts/generated.rs", "mod helper;\n")
        .file("src/helper.rs", "fn h() {}\n")
        .file("src/template.rs", "fn t() {}\n")
        .file(
            "src/data.rs",
            "const T: &str = include_str!(\"template.rs\");\n",
        );

    // `data.rs` is itself unreached; `template.rs` is reached only as data.
    assert_eq!(fixture.orphans(), ["src/data.rs", "src/template.rs"]);

    let with_data = demo()
        .file(
            "src/lib.rs",
            "const T: &str = include_str!(\"template.rs\");\n",
        )
        .file("src/template.rs", "fn t() {}\n");
    assert_eq!(with_data.orphans(), Vec::<String>::new());
}

#[test]
fn a_package_with_a_computed_include_is_not_searched() {
    let fixture = demo()
        .file(
            "src/lib.rs",
            "include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/x.rs\"));\n",
        )
        .file("src/x.rs", "fn x() {}\n");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn a_name_a_macro_might_declare_is_not_an_orphan() {
    let fixture = demo()
        .file("src/lib.rs", "declare_modules!(generated);\n")
        .file("src/generated.rs", "fn g() {}\n")
        .file("src/stale/mod.rs", "fn s() {}\n");

    assert_eq!(fixture.orphans(), ["src/stale/mod.rs"]);
}

#[test]
fn a_package_with_an_unreadable_reachable_file_is_not_searched() {
    let fixture = demo()
        .file("src/lib.rs", "mod huge;\n")
        .unparsed("src/huge.rs")
        .file("src/x.rs", "fn x() {}\n");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn a_package_whose_root_the_walk_excluded_is_not_searched() {
    let fixture = demo()
        .excluded("src/lib.rs")
        .file("src/x.rs", "fn x() {}\n");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn a_package_with_no_root_at_all_is_not_searched() {
    let fixture = demo().file("src/x.rs", "fn x() {}\n");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn files_without_items_or_outside_src_are_not_reported() {
    let fixture = demo()
        .file("src/lib.rs", "fn l() {}\n")
        .file("src/empty.rs", "//! Nothing here yet.\n")
        .file("tools/script.rs", "fn s() {}\n")
        .unparsed("src/broken.rs");

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}

#[test]
fn a_dangling_declaration_is_ignored() {
    let fixture = demo()
        .file("src/lib.rs", "mod missing;\n")
        .file("src/x.rs", "fn x() {}\n");

    assert_eq!(fixture.orphans(), ["src/x.rs"]);
}

#[test]
fn a_nested_package_owns_its_own_src() {
    let fixture = demo()
        .package("src/inner", "inner", "[package]\nname = \"inner\"\n")
        .file("src/lib.rs", "fn l() {}\n")
        .file("src/inner/src/lib.rs", "fn i() {}\n")
        .file("src/inner/src/lost.rs", "fn lost() {}\n");

    let tree = fixture.resolve();

    assert_eq!(tree.orphans.len(), 1);
    assert_eq!(tree.orphans[0].path, "src/inner/src/lost.rs");
    assert_eq!(tree.orphans[0].crate_name, "inner");
}

#[test]
fn a_manifest_path_above_the_root_is_ignored() {
    let fixture = Fixture::new()
        .package(
            ".",
            "demo",
            "[package]\nname = \"demo\"\n[lib]\npath = \"../outside.rs\"\n",
        )
        .file("src/lib.rs", "fn l() {}\n")
        .file("src/x.rs", "fn x() {}\n");

    assert_eq!(fixture.orphans(), ["src/x.rs"]);
}

#[test]
fn a_file_another_package_reaches_is_not_an_orphan() {
    let fixture = Fixture::new()
        .package("a", "a", "[package]\nname = \"a\"\n")
        .package("b", "b", "[package]\nname = \"b\"\n")
        .file("a/src/lib.rs", "fn a() {}\n")
        .file("a/src/shared.rs", "pub fn s() {}\n")
        .file(
            "b/src/lib.rs",
            "#[path = \"../../a/src/shared.rs\"]\nmod shared;\n",
        );

    assert_eq!(fixture.orphans(), Vec::<String>::new());
}
