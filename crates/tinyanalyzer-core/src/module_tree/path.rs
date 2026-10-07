//! Lexical path arithmetic over root-relative, forward-slash paths.
//!
//! The module tree never touches the filesystem to resolve a path: every path
//! it compares is one the walk already produced, so normalizing by text is both
//! sufficient and the only way two spellings of one file compare equal.

/// Joins `relative` onto the directory `base`, folding `.` and `..`.
///
/// `base` is `"."` for the analysis root. Returns `None` for an absolute
/// `relative` or one that climbs above the analysis root: neither can name a
/// file the walk found.
pub(crate) fn join(base: &str, relative: &str) -> Option<String> {
    let relative = relative.replace('\\', "/");
    if relative.starts_with('/') {
        return None;
    }

    let mut parts: Vec<&str> = Vec::new();
    for part in base.split('/').chain(relative.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }

    Some(parts.join("/"))
}

/// The directory holding `path`, or `"."` for a file at the root.
pub(crate) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or(".", |(parent, _)| parent)
}

/// The file name of `path` without its `.rs` extension.
pub(crate) fn stem(path: &str) -> &str {
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    name.strip_suffix(".rs").unwrap_or(name)
}
