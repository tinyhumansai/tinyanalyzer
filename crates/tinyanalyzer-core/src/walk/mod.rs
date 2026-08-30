//! Deciding which files an analysis covers, and reading them.
//!
//! Traversal is delegated to the `ignore` crate — ripgrep's walker — so
//! "analyze this repository" means the same set of files a developer sees in
//! their editor, `.gitignore` semantics and all, rather than every byte on
//! disk. Layered on top of that are the include, exclude, and test globs from
//! [`ScanConfig`], which are matched against the path *relative to the analysis
//! root*: a configuration file has no business knowing where the repository is
//! checked out.
//!
//! Contents are read here rather than lazily by each analyzer. Every downstream
//! pass needs the text, reading is the dominant cost on a large tree, and doing
//! it once in a parallel walk is what keeps a whole-repository analysis in the
//! range where an operator will actually wait for it.

mod types;

pub use types::SourceFile;

use crate::config::{ScanConfig, compile_glob_set};
use crate::error::{Error, Result};
use crate::loc::Language;
use ignore::{WalkBuilder, WalkState};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

/// Walks `root` and returns every file the configuration admits.
///
/// Results are sorted by path, so two runs over an unchanged tree produce
/// byte-identical reports. That is not cosmetic: a report that reorders itself
/// run to run cannot be diffed, and diffing two reports is how you see what a
/// refactor actually did.
///
/// # Errors
///
/// Returns [`Error::RootNotADirectory`] if `root` is not a directory,
/// [`Error::Glob`] if any configured pattern is not a valid glob, and
/// [`Error::Walk`] if traversal itself fails.
pub fn discover(root: impl AsRef<Path>, scan: &ScanConfig) -> Result<Vec<SourceFile>> {
    let root = root.as_ref();
    if !root.is_dir() {
        return Err(Error::RootNotADirectory {
            path: root.to_path_buf(),
        });
    }

    let include = compile_glob_set(&scan.include)?;
    let exclude = compile_glob_set(&scan.exclude)?;
    let tests = compile_glob_set(&scan.test_patterns)?;

    let collected = Mutex::new(Vec::new());
    let failure = Mutex::new(None);

    WalkBuilder::new(root)
        .hidden(!scan.include_hidden)
        .git_ignore(scan.respect_gitignore)
        .git_global(scan.respect_gitignore)
        .git_exclude(scan.respect_gitignore)
        .ignore(scan.respect_gitignore)
        .parents(scan.respect_gitignore)
        // `.gitignore` files are honored even outside a git repository. A
        // worktree, an export, or an unpacked archive still carries the ignore
        // rules the project wrote, and an analysis that silently included
        // `target/` because `.git` was absent would be reporting a build
        // directory as source.
        .require_git(false)
        .follow_links(scan.follow_symlinks)
        .build_parallel()
        .run(|| {
            Box::new(|entry| {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        // One unreadable directory should not lose the whole
                        // analysis, but it must not be silent either: the first
                        // failure is kept and reported once the walk finishes.
                        let mut slot = failure.lock().unwrap_or_else(PoisonError::into_inner);
                        if slot.is_none() {
                            *slot = Some(error.to_string());
                        }
                        return WalkState::Continue;
                    }
                };

                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                    return WalkState::Continue;
                }

                let relative = relative_path(root, entry.path());

                if exclude.as_ref().is_some_and(|set| set.is_match(&relative)) {
                    return WalkState::Continue;
                }
                if include.as_ref().is_some_and(|set| !set.is_match(&relative)) {
                    return WalkState::Continue;
                }

                let is_test_path = tests.as_ref().is_some_and(|set| set.is_match(&relative));
                let file = read_file(entry.path(), relative, is_test_path, scan.max_file_bytes);

                collected
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(file);

                WalkState::Continue
            })
        });

    if let Some(message) = failure.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(Error::Walk {
            root: root.to_path_buf(),
            message,
        });
    }

    let mut files = collected
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(files)
}

/// Renders `path` relative to `root` with forward slashes.
///
/// A path that is not under `root` is returned as it stands. That can only
/// happen behind a followed symbolic link, and naming the file by the path the
/// walk actually reached it through is more useful than dropping it.
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join("/")
}

/// Reads one file, or records why it was left unread.
///
/// A read failure is not an error for the analysis as a whole: a file that
/// disappeared between the walk and the read, or one the process cannot open,
/// still belongs in the report with the size the walker saw.
fn read_file(
    absolute: &Path,
    relative_path: String,
    is_test_path: bool,
    max_file_bytes: u64,
) -> SourceFile {
    let language = Language::from_file_name(&relative_path);
    let bytes = std::fs::metadata(absolute).map_or(0, |meta| meta.len());

    let text = if bytes > max_file_bytes {
        None
    } else {
        std::fs::read_to_string(absolute).ok()
    };

    SourceFile {
        relative_path,
        absolute_path: absolute.to_path_buf(),
        language,
        bytes,
        text,
        is_test_path,
    }
}

#[cfg(test)]
mod test;
