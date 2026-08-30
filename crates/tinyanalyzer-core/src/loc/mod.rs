//! Counting the lines of a source file.
//!
//! This is the one measurement every other part of the report leans on, so it
//! is worth being precise about what it means. A line is classified exactly
//! once, into code, comment, or blank:
//!
//! - **blank** — empty, or whitespace only.
//! - **comment** — the whole line is comment, whether from a line-comment
//!   prefix or because it falls inside a block comment.
//! - **code** — anything else, including a line of code with a comment on the
//!   end of it. That line has to be read to understand the program, so it
//!   counts as code.
//!
//! The counter is a scanner, not a parser: it tracks block-comment nesting and
//! ignores delimiters inside string literals, which covers real source without
//! the cost of parsing every language in the tree. For Rust specifically the
//! parsed analysis in [`crate::rust_source`] is the authority on structure;
//! this module only ever counts lines.
//!
//! # Example
//!
//! ```
//! use tinyanalyzer_core::{Language, count_lines};
//!
//! let counts = count_lines(
//!     Language::Rust,
//!     "// what this does\nfn main() {\n\n    run(); // go\n}\n",
//! );
//!
//! assert_eq!(counts.total, 5);
//! assert_eq!(counts.comment, 1);
//! assert_eq!(counts.blank, 1);
//! assert_eq!(counts.code, 3);
//! ```

mod types;

pub use types::{Language, LineCounts};

/// Counts the lines of `text` using `language`'s comment syntax.
///
/// A trailing newline does not create an extra line: `"a\n"` is one line, the
/// same as `"a"`. That matches what an editor shows and what every other line
/// count in the report is compared against.
#[must_use]
pub fn count_lines(language: Language, text: &str) -> LineCounts {
    let mut counts = LineCounts::default();
    let mut open_block: Option<&'static str> = None;

    for line in text.lines() {
        counts.total = counts.total.saturating_add(1);

        let trimmed = line.trim();
        if trimmed.is_empty() && open_block.is_none() {
            counts.blank = counts.blank.saturating_add(1);
            continue;
        }

        let classification = classify(language, trimmed, &mut open_block);
        match classification {
            LineKind::Code => counts.code = counts.code.saturating_add(1),
            LineKind::Comment => counts.comment = counts.comment.saturating_add(1),
            LineKind::Blank => counts.blank = counts.blank.saturating_add(1),
        }
    }

    counts
}

/// What a single line turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Code,
    Comment,
    Blank,
}

/// Classifies one already-trimmed line, advancing block-comment state.
///
/// `open_block` holds the closing delimiter the scanner is currently looking
/// for, or `None` when it is not inside a block comment. It is threaded through
/// rather than recomputed because a block comment is the one piece of state a
/// line-at-a-time scanner cannot recover from the line alone.
fn classify(language: Language, trimmed: &str, open_block: &mut Option<&'static str>) -> LineKind {
    // Inside a block comment, the only question is whether it ends here and
    // whether anything follows the terminator on the same line. This is tested
    // before the blank check on purpose: an empty line between `/*` and `*/` is
    // part of the comment, not a blank line that happens to sit inside one.
    if let Some(close) = *open_block {
        return match trimmed.find(close) {
            None => LineKind::Comment,
            Some(at) => {
                *open_block = None;
                let rest = &trimmed[at.saturating_add(close.len())..];
                match classify(language, rest.trim(), open_block) {
                    LineKind::Code => LineKind::Code,
                    LineKind::Comment | LineKind::Blank => LineKind::Comment,
                }
            }
        };
    }

    if trimmed.is_empty() {
        return LineKind::Blank;
    }

    if language
        .line_comments()
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return LineKind::Comment;
    }

    match first_block_open(language, trimmed) {
        Some((0, open, close)) => {
            let after = &trimmed[open.len()..];
            match after.find(close) {
                None => {
                    *open_block = Some(close);
                    LineKind::Comment
                }
                Some(end) => {
                    let rest = &after[end.saturating_add(close.len())..];
                    match classify(language, rest.trim(), open_block) {
                        LineKind::Code => LineKind::Code,
                        LineKind::Comment | LineKind::Blank => LineKind::Comment,
                    }
                }
            }
        }
        // A block comment that opens part-way through a line leaves code before
        // it, so the line is code — but the comment may still run on, and the
        // scanner has to know that for the lines that follow.
        Some((at, open, close)) => {
            let after = &trimmed[at.saturating_add(open.len())..];
            if !after.contains(close) {
                *open_block = Some(close);
            }
            LineKind::Code
        }
        None => LineKind::Code,
    }
}

/// Finds the earliest block-comment opener on a line that is not inside a
/// string literal.
///
/// Skipping string literals is what keeps a line like `print("/* hi */")` from
/// putting the scanner into a comment it never leaves.
fn first_block_open(language: Language, line: &str) -> Option<(usize, &'static str, &'static str)> {
    let delimiters = language.block_comments();
    if delimiters.is_empty() {
        return None;
    }

    let bytes = line.as_bytes();
    let mut index = 0;
    let mut in_string: Option<u8> = None;

    while index < bytes.len() {
        let byte = bytes[index];

        if let Some(quote) = in_string {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == quote {
                in_string = None;
            }
            index = index.saturating_add(1);
            continue;
        }

        // Python's block delimiters *are* string quotes, so a language whose
        // comments are triple-quoted strings cannot also skip string literals.
        let quotes_are_comments = delimiters
            .iter()
            .any(|(open, _)| open.starts_with('"') || open.starts_with('\''));

        if !quotes_are_comments && (byte == b'"' || byte == b'\'') {
            in_string = Some(byte);
            index = index.saturating_add(1);
            continue;
        }

        for (open, close) in delimiters {
            if line[index..].starts_with(open) {
                return Some((index, open, close));
            }
        }

        index = index.saturating_add(1);
    }

    None
}

#[cfg(test)]
mod test;
