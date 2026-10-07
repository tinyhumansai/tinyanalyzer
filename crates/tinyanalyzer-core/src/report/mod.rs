//! Assembling one analysis into a report.
//!
//! This is the module that runs the others in order and joins their answers up:
//! walk the tree, parse every Rust file, resolve the dependency graph, take the
//! symbol census, apply the rules, and aggregate the lot into a [`Report`].
//!
//! Two choices here are worth stating, because they are what the rest of the
//! crate is arranged around.
//!
//! **Parsing is parallel and everything else is not.** Parsing dominates the
//! wall-clock time of an analysis by an order of magnitude, and it is
//! embarrassingly parallel — each file is independent. Aggregation is cheap and
//! order-sensitive, so it stays sequential, where it is easy to be sure it is
//! right.
//!
//! **Crate ownership is read from manifests already on disk, not from a second
//! `cargo metadata` call.** The walk has already read every `Cargo.toml` in the
//! tree, and mapping a file to the nearest manifest above it is exactly the rule
//! cargo itself uses. Spending another process launch to learn something already
//! in hand would double the slowest part of a run that does not need the graph.

mod types;

pub use types::{
    DirectoryMetrics, FileMetrics, LanguageMetrics, ParseFailureReport, ProjectSummary, Report,
    SCHEMA_VERSION, Totals,
};

use crate::clones::{self, CloneInput, SymbolRecord};
use crate::config::{CloneConfig, Config, ScanConfig, compile_glob_set};
use crate::dead_code::DeadCodeInput;
use crate::deps::{self, CrateReferences, DependencyReport};
use crate::error::{Error, Result};
use crate::findings::{self, FindingInputs};
use crate::loc::{Language, LineCounts, count_lines};
use crate::module_tree::{self, FileDeclarations, ModuleInput, ModuleTree, PackageManifest};
use crate::rust_source::{self, RustFile};
use crate::walk::{self, SourceFile};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Analyzes the repository rooted at `root`, loading its configuration.
///
/// # Errors
///
/// Returns [`Error::RootNotADirectory`] if `root` is not a directory,
/// [`Error::Config`] or [`Error::Io`] if the configuration cannot be read, and
/// [`Error::Glob`] or [`Error::Walk`] if the walk fails. A dependency-graph
/// failure is *not* propagated: see [`analyze_with`].
pub fn analyze(root: impl AsRef<Path>) -> Result<Report> {
    let root = root.as_ref();
    let config = Config::load(root)?;

    analyze_with(root, &config)
}

/// Analyzes the repository rooted at `root` with an explicit configuration.
///
/// A failure to resolve the dependency graph leaves [`Report::dependencies`]
/// empty rather than failing the analysis. Cargo not resolving is a normal state
/// for a tree mid-refactor, and the file-level half of the report — which is
/// most of it — is still worth having.
///
/// # Errors
///
/// Returns [`Error::RootNotADirectory`] if `root` is not a directory, and
/// [`Error::Glob`] or [`Error::Walk`] if the walk fails.
pub fn analyze_with(root: impl AsRef<Path>, config: &Config) -> Result<Report> {
    let root = root.as_ref();
    if !root.is_dir() {
        return Err(Error::RootNotADirectory {
            path: root.to_path_buf(),
        });
    }

    // Resolved before anything reads it, so that analyzing `.` reports the
    // directory's real name rather than a project called "unnamed project".
    // A root that cannot be canonicalized is still analyzable, so the failure
    // falls back rather than propagating.
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let root = canonical.as_path();

    let discovered = walk::discover(root, &config.scan)?;
    let manifests = manifest_map(&discovered);

    let mut parsed: Vec<ParsedFile<'_>> = discovered
        .par_iter()
        .map(|file| parse_one(file, &manifests))
        .collect();

    let tree = resolve_modules(root, &parsed, &manifests);
    mark_test_only(&mut parsed, &tree.test_only);
    let orphaned: BTreeSet<&str> = tree
        .orphans
        .iter()
        .map(|orphan| orphan.path.as_str())
        .collect();

    let dependencies = if config.dependencies.enabled {
        deps::analyze(root, &config.dependencies, &crate_references(&parsed)).unwrap_or_default()
    } else {
        DependencyReport::default()
    };

    let mut files: Vec<FileMetrics> = parsed
        .iter()
        .map(|file| measurements(file, config))
        .collect();

    files.sort_by(|left, right| {
        right
            .weight
            .partial_cmp(&left.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
    });

    // An orphaned file is not compiled, so nothing in it is a use of anything.
    let dead_inputs: Vec<DeadCodeInput<'_>> = parsed
        .iter()
        .filter(|file| !orphaned.contains(file.source.relative_path.as_str()))
        .filter_map(|file| {
            file.rust.as_ref().map(|rust| DeadCodeInput {
                path: file.source.relative_path.as_str(),
                rust,
                is_test_file: file.is_test,
            })
        })
        .collect();
    let mut dead_code = crate::dead_code::analyze(&dead_inputs, &config.dead_code);
    if config.dead_code.enabled {
        crate::dead_code::add_orphans(&mut dead_code, &tree.orphans, |path| {
            parsed
                .iter()
                .any(|file| file.is_test && file.source.relative_path == path)
        });
    }

    let clones = if config.clones.enabled {
        let extra = extra_sources(root, config)?;
        let inputs = clone_inputs(&discovered, &extra, &config.clones, &tree)?;
        clones::analyze(&inputs, &config.clones, &config.thresholds)
    } else {
        Vec::new()
    };

    let parse_failures: Vec<ParseFailureReport> = parsed
        .iter()
        .filter_map(|file| file.failure.clone())
        .collect();

    let directories = directory_metrics(&files);
    let languages = language_metrics(&files);
    let totals = totals(&files, &dependencies, &directories);

    let findings = findings::analyze(
        FindingInputs {
            files: &files,
            directories: &directories,
            dependencies: &dependencies,
            dead_code: &dead_code,
            parse_failures: &parse_failures,
            clones: &clones,
        },
        &config.thresholds,
    );

    Ok(Report {
        schema_version: SCHEMA_VERSION,
        project: ProjectSummary {
            name: config.display_name(root),
            description: config.project.description.clone(),
            root: root.display().to_string(),
            generated_at_unix: now_unix(),
            config_file: Config::locate(root).map(|path| path.display().to_string()),
        },
        totals,
        files,
        directories,
        languages,
        dependencies,
        dead_code,
        clones,
        findings,
        parse_failures,
    })
}

/// Seconds since the Unix epoch, or zero if the clock is unreadable.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// One file after parsing, before aggregation.
struct ParsedFile<'a> {
    source: &'a SourceFile,
    lines: LineCounts,
    test_lines: LineCounts,
    rust: Option<RustFile>,
    failure: Option<ParseFailureReport>,
    crate_name: Option<String>,
    is_test: bool,
    declarations: Option<FileDeclarations>,
}

/// Counts and parses one file.
fn parse_one<'a>(source: &'a SourceFile, manifests: &BTreeMap<String, Manifest>) -> ParsedFile<'a> {
    let text = source.text.as_deref().unwrap_or_default();
    let lines = count_lines(source.language, text);

    let (rust, declarations, failure) =
        if source.language == Language::Rust && source.text.is_some() {
            match rust_source::analyze_with_declarations(text) {
                Ok((parsed, declarations)) => (Some(parsed), Some(declarations), None),
                Err(error) => (
                    None,
                    None,
                    Some(ParseFailureReport {
                        path: source.relative_path.clone(),
                        line: error.line,
                        message: error.message,
                    }),
                ),
            }
        } else {
            (None, None, None)
        };

    let is_test = source.is_test_path || rust.as_ref().is_some_and(|file| file.is_test_module);
    let test_lines = if is_test {
        lines
    } else {
        rust.as_ref().map_or_else(LineCounts::default, |rust| {
            counts_for_ranges(text, &rust.test_ranges)
        })
    };

    ParsedFile {
        crate_name: owning_crate(&source.relative_path, manifests),
        source,
        lines,
        test_lines,
        rust,
        failure,
        is_test,
        declarations,
    }
}

/// Resolves every package's module tree over the parsed files.
fn resolve_modules(
    root: &Path,
    parsed: &[ParsedFile<'_>],
    manifests: &BTreeMap<String, Manifest>,
) -> ModuleTree {
    let packages: Vec<PackageManifest<'_>> = manifests
        .iter()
        .map(|(directory, manifest)| PackageManifest {
            directory,
            name: &manifest.name,
            manifest: &manifest.value,
        })
        .collect();
    let files: Vec<ModuleInput<'_>> = parsed
        .iter()
        .filter(|file| file.source.language == Language::Rust)
        .map(|file| ModuleInput {
            path: &file.source.relative_path,
            declarations: file.declarations.as_ref(),
        })
        .collect();

    module_tree::resolve(&packages, &files, &|path| root.join(path).is_file())
}

/// Marks every file only test code reaches as test code, whole.
fn mark_test_only(parsed: &mut [ParsedFile<'_>], test_only: &BTreeSet<String>) {
    for file in parsed {
        if !file.is_test && test_only.contains(&file.source.relative_path) {
            file.is_test = true;
            file.test_lines = file.lines;
        }
    }
}

/// Turns one parsed file into its reported measurements.
fn measurements(file: &ParsedFile<'_>, config: &Config) -> FileMetrics {
    FileMetrics {
        path: file.source.relative_path.clone(),
        directory: file.source.directory().to_owned(),
        language: file.source.language,
        bytes: file.source.bytes,
        lines: file.lines,
        test_lines: file.test_lines,
        is_test: file.is_test,
        crate_name: file.crate_name.clone(),
        weight: weight(file.lines, file.rust.as_ref()),
        notes: config
            .notes_for(&file.source.relative_path)
            .unwrap_or_default()
            .into_iter()
            .cloned()
            .collect(),
        rust: file.rust.clone(),
    }
}

/// Counts only source lines covered by the supplied inclusive ranges.
fn counts_for_ranges(text: &str, ranges: &[rust_source::SourceRange]) -> LineCounts {
    if ranges.is_empty() {
        return LineCounts::default();
    }
    let mut selected = String::new();
    for (index, line) in text.lines().enumerate() {
        let number = index.saturating_add(1);
        if ranges
            .iter()
            .any(|range| number >= range.start_line && number <= range.end_line)
        {
            selected.push_str(line);
            selected.push('\n');
        }
    }
    count_lines(Language::Rust, &selected)
}

/// How heavy one file is, as a single comparable number.
///
/// Lines alone rank a long list of trivial constants above a dense two-hundred
/// line state machine, which is the wrong answer to "what should I look at
/// first". So the score is lines of code, plus:
///
/// - **three per unit of branching** beyond the one path every function has,
/// - **five per level of nesting** beyond three,
/// - **ten per allocation inside a loop**.
///
/// The weights are chosen so that a file has to be substantially more complex,
/// not marginally so, to outrank a file twice its length. The number means
/// nothing on its own; it exists to order a list.
#[must_use]
pub fn weight(lines: LineCounts, rust: Option<&RustFile>) -> f64 {
    // Every input is a line or item count from a single file, far below the
    // range where `f64` loses integer precision.
    #[allow(clippy::cast_precision_loss)]
    let base = lines.code as f64;

    let Some(rust) = rust else {
        return base;
    };

    let branching: u32 = rust
        .functions
        .iter()
        .map(|function| function.complexity.saturating_sub(1))
        .sum();
    let excess_nesting = rust.max_nesting.saturating_sub(3);

    #[allow(clippy::cast_precision_loss)]
    {
        base + 3.0 * f64::from(branching)
            + 5.0 * excess_nesting as f64
            + 10.0 * rust.performance.allocations_in_loops as f64
    }
}

/// A package manifest the walk loaded.
struct Manifest {
    /// The package's name.
    name: String,
    /// The whole parsed manifest, for its target paths.
    value: toml::Value,
}

/// Maps every manifest directory in the tree to the package it declares.
///
/// Reads the `name` out of each `Cargo.toml` the walk already loaded. A
/// manifest that is a virtual workspace root declares no package and is skipped,
/// which is what makes files at the repository root come back with no owning
/// crate rather than with the workspace's name.
fn manifest_map(files: &[SourceFile]) -> BTreeMap<String, Manifest> {
    let mut manifests = BTreeMap::new();

    for file in files {
        if file.file_name() != "Cargo.toml" {
            continue;
        }

        let Some(text) = &file.text else {
            continue;
        };
        let Ok(parsed) = toml::from_str::<toml::Value>(text) else {
            continue;
        };
        let Some(name) = parsed
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };

        manifests.insert(
            file.directory().to_owned(),
            Manifest {
                name: name.to_owned(),
                value: parsed.clone(),
            },
        );
    }

    manifests
}

/// The crate that owns `path`: the nearest manifest at or above it.
fn owning_crate(path: &str, manifests: &BTreeMap<String, Manifest>) -> Option<String> {
    let mut directory = match path.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => ".",
    };

    loop {
        if let Some(manifest) = manifests.get(directory) {
            return Some(manifest.name.clone());
        }

        match directory.rsplit_once('/') {
            Some((parent, _)) => directory = parent,
            None if directory == "." => return None,
            None => directory = ".",
        }
    }
}

/// Which crate names each workspace member's source files mention.
fn crate_references(files: &[ParsedFile<'_>]) -> CrateReferences {
    let mut references: CrateReferences = BTreeMap::new();

    for file in files {
        let (Some(crate_name), Some(rust)) = (&file.crate_name, &file.rust) else {
            continue;
        };

        let entry = references.entry(crate_name.clone()).or_default();
        for referenced in &rust.referenced_crates {
            entry.insert(deps::normalize_crate_name(referenced));
        }
    }

    references
}

/// Aggregates files into per-directory rows, largest first.
fn directory_metrics(files: &[FileMetrics]) -> Vec<DirectoryMetrics> {
    let mut by_path: BTreeMap<&str, DirectoryMetrics> = BTreeMap::new();

    for file in files {
        let entry = by_path
            .entry(file.directory.as_str())
            .or_insert_with(|| DirectoryMetrics {
                path: file.directory.clone(),
                files: 0,
                bytes: 0,
                lines: LineCounts::default(),
                is_test_only: true,
            });

        entry.files = entry.files.saturating_add(1);
        entry.bytes = entry.bytes.saturating_add(file.bytes);
        entry.lines.add(file.lines);
        entry.is_test_only = entry.is_test_only && file.is_test;
    }

    let mut directories: Vec<DirectoryMetrics> = by_path.into_values().collect();
    directories.sort_by(|left, right| {
        right
            .lines
            .code
            .cmp(&left.lines.code)
            .then_with(|| left.path.cmp(&right.path))
    });

    directories
}

/// Aggregates files into per-language rows, largest first.
fn language_metrics(files: &[FileMetrics]) -> Vec<LanguageMetrics> {
    let mut by_language: BTreeMap<Language, LanguageMetrics> = BTreeMap::new();

    for file in files {
        let entry = by_language
            .entry(file.language)
            .or_insert_with(|| LanguageMetrics {
                language: file.language,
                files: 0,
                bytes: 0,
                lines: LineCounts::default(),
            });

        entry.files = entry.files.saturating_add(1);
        entry.bytes = entry.bytes.saturating_add(file.bytes);
        entry.lines.add(file.lines);
    }

    let mut languages: Vec<LanguageMetrics> = by_language.into_values().collect();
    languages.sort_by(|left, right| {
        right
            .lines
            .code
            .cmp(&left.lines.code)
            .then_with(|| left.language.cmp(&right.language))
    });

    languages
}

/// Sums every file into whole-repository totals.
fn totals(
    files: &[FileMetrics],
    dependencies: &DependencyReport,
    directories: &[DirectoryMetrics],
) -> Totals {
    let mut totals = Totals::default();

    for file in files {
        totals.absorb(file);
    }

    totals.directories = directories.len();
    totals.packages = dependencies.packages.len();
    totals.external_packages = dependencies.external_packages;

    totals
}

/// The symbol index for the repository rooted at `root`: every item in every
/// Rust file the analysis would cover, including the configured extra roots.
///
/// # Errors
///
/// Returns [`Error::RootNotADirectory`] if `root` is not a directory, and
/// [`Error::Glob`] or [`Error::Walk`] if a walk fails or a `read_only` pattern
/// is not a valid glob.
pub fn symbol_index(root: impl AsRef<Path>, config: &Config) -> Result<Vec<SymbolRecord>> {
    let root = root.as_ref();
    if !root.is_dir() {
        return Err(Error::RootNotADirectory {
            path: root.to_path_buf(),
        });
    }
    let discovered = walk::discover(root, &config.scan)?;
    let extra = extra_sources(root, config)?;
    let inputs = clone_inputs(&discovered, &extra, &config.clones, &ModuleTree::default())?;
    Ok(clones::symbols(&inputs))
}

/// Files under the configured extra roots, renamed to carry the root as a
/// prefix, with whether each root is editable.
fn extra_sources(root: &Path, config: &Config) -> Result<Vec<(bool, SourceFile)>> {
    let mut sources = Vec::new();
    for extra in &config.clones.extra_roots {
        let prefix = extra.path.trim_end_matches(['/', '\\']).replace('\\', "/");
        for mut file in walk::discover(root.join(&extra.path), &ScanConfig::default())? {
            file.relative_path = format!("{prefix}/{}", file.relative_path);
            sources.push((extra.editable, file));
        }
    }
    Ok(sources)
}

/// The Rust files clone detection reads, with their editability resolved.
///
/// Orphaned files are left out: a file nothing compiles is not a second copy
/// of anything, it is a file to delete, and the `orphan_file` finding says so.
/// A file the module tree found only test code reaching is passed as test code.
fn clone_inputs<'a>(
    discovered: &'a [SourceFile],
    extra: &'a [(bool, SourceFile)],
    config: &CloneConfig,
    tree: &ModuleTree,
) -> Result<Vec<CloneInput<'a>>> {
    let read_only = compile_glob_set(&config.read_only)?;
    let writable = |path: &str| read_only.as_ref().is_none_or(|globs| !globs.is_match(path));

    let own = discovered.iter().map(|file| (true, file));
    let extra = extra.iter().map(|(editable, file)| (*editable, file));
    Ok(own
        .chain(extra)
        .filter(|(_, file)| file.language == Language::Rust)
        .filter(|(_, file)| {
            !tree
                .orphans
                .iter()
                .any(|orphan| orphan.path == file.relative_path)
        })
        .filter_map(|(editable, file)| {
            Some(CloneInput {
                path: file.relative_path.as_str(),
                text: file.text.as_deref()?,
                is_test_path: file.is_test_path || tree.test_only.contains(&file.relative_path),
                editable: editable && writable(&file.relative_path),
            })
        })
        .collect())
}

#[cfg(test)]
mod test;
