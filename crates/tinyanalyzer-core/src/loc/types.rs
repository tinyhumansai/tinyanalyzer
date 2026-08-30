//! Languages the counter recognizes and the shape of a count.

use serde::{Deserialize, Serialize};

/// A source language, identified from a file's extension or name.
///
/// The variant set is not "every language that exists" — it is every language
/// whose comment syntax the counter knows. Anything else is [`Language::Other`]
/// and gets line counts without a code/comment split, which is honest rather
/// than wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Language {
    /// Rust source. The only language this crate also parses.
    Rust,
    /// TOML, including every `Cargo.toml`.
    Toml,
    /// Markdown prose.
    Markdown,
    /// JSON data.
    Json,
    /// YAML data.
    Yaml,
    /// Shell script.
    Shell,
    /// JavaScript, including JSX.
    JavaScript,
    /// TypeScript, including TSX.
    TypeScript,
    /// HTML markup.
    Html,
    /// CSS stylesheets.
    Css,
    /// Python source.
    Python,
    /// C, C++, and their headers.
    C,
    /// Go source.
    Go,
    /// SQL.
    Sql,
    /// A recognized text file with no known comment syntax.
    Other,
}

impl Language {
    /// The name shown on the dashboard.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Shell => "Shell",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Python => "Python",
            Self::C => "C/C++",
            Self::Go => "Go",
            Self::Sql => "SQL",
            Self::Other => "Other",
        }
    }

    /// Line-comment prefixes for this language.
    ///
    /// A line whose first non-whitespace characters are one of these is a
    /// comment line.
    #[must_use]
    pub const fn line_comments(self) -> &'static [&'static str] {
        match self {
            Self::Rust | Self::JavaScript | Self::TypeScript | Self::C | Self::Go | Self::Css => {
                &["//"]
            }
            Self::Toml | Self::Yaml | Self::Shell | Self::Python => &["#"],
            Self::Sql => &["--"],
            // JSON has no comments, Markdown's are HTML blocks, and `Other` is
            // by definition unknown.
            Self::Json | Self::Markdown | Self::Html | Self::Other => &[],
        }
    }

    /// Block-comment delimiters for this language, as `(open, close)` pairs.
    #[must_use]
    pub const fn block_comments(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Rust
            | Self::JavaScript
            | Self::TypeScript
            | Self::C
            | Self::Go
            | Self::Css
            | Self::Sql => &[("/*", "*/")],
            Self::Python => &[("\"\"\"", "\"\"\""), ("'''", "'''")],
            Self::Html | Self::Markdown => &[("<!--", "-->")],
            Self::Toml | Self::Yaml | Self::Shell | Self::Json | Self::Other => &[],
        }
    }

    /// Whether this language's code/comment split is meaningful.
    ///
    /// For a language with no known comment syntax every non-blank line counts
    /// as code, which would otherwise read as "perfectly undocumented" on the
    /// dashboard.
    #[must_use]
    pub const fn has_comment_syntax(self) -> bool {
        !self.line_comments().is_empty() || !self.block_comments().is_empty()
    }

    /// Identifies a language from a file name.
    ///
    /// Matches on the extension first and falls back to well-known whole file
    /// names, which is how `Makefile` and `Dockerfile` are recognized at all.
    #[must_use]
    pub fn from_file_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();

        let extension = lower.rsplit_once('.').map_or("", |(_, ext)| ext);

        match extension {
            "rs" => Self::Rust,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            "json" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "sh" | "bash" | "zsh" => Self::Shell,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "tsx" => Self::TypeScript,
            "html" | "htm" => Self::Html,
            "css" | "scss" | "sass" => Self::Css,
            "py" => Self::Python,
            "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" => Self::C,
            "go" => Self::Go,
            "sql" => Self::Sql,
            _ => Self::Other,
        }
    }
}

/// How the lines of one file divide up.
///
/// `code + comment + blank == total` always holds: a line is classified exactly
/// once, and a line carrying both code and a trailing comment counts as code,
/// because that is the line you would have to read to understand the program.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineCounts {
    /// Every line in the file.
    pub total: usize,
    /// Lines carrying at least one non-comment token.
    pub code: usize,
    /// Lines that are entirely comment.
    pub comment: usize,
    /// Lines that are empty or whitespace only.
    pub blank: usize,
}

impl LineCounts {
    /// Comment lines as a fraction of code lines.
    ///
    /// Returns `0.0` for a file with no code, which keeps the value finite for
    /// a pure documentation file rather than reporting an infinite ratio.
    #[must_use]
    pub fn comment_ratio(self) -> f64 {
        if self.code == 0 {
            return 0.0;
        }

        // Both counts are line numbers within one file, far below the range
        // where `f64` loses integer precision.
        #[allow(clippy::cast_precision_loss)]
        {
            self.comment as f64 / self.code as f64
        }
    }

    /// Adds another file's counts into this one.
    ///
    /// Saturating rather than wrapping: a total that silently wrapped to zero
    /// would be reported as a clean repository.
    pub fn add(&mut self, other: Self) {
        self.total = self.total.saturating_add(other.total);
        self.code = self.code.saturating_add(other.code);
        self.comment = self.comment.saturating_add(other.comment);
        self.blank = self.blank.saturating_add(other.blank);
    }
}
