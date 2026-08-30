//! Unit tests for line counting and language identification.
//!
//! Every case asserts the invariant `code + comment + blank == total` alongside
//! the split it is actually about, because a classifier that loses or
//! double-counts a line is the failure mode that would silently skew every
//! total in the report.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Language, LineCounts, count_lines};

fn counted(language: Language, text: &str) -> LineCounts {
    let counts = count_lines(language, text);

    assert_eq!(
        counts.code + counts.comment + counts.blank,
        counts.total,
        "every line must be classified exactly once"
    );

    counts
}

#[test]
fn an_empty_file_counts_nothing() {
    assert_eq!(counted(Language::Rust, ""), LineCounts::default());
}

#[test]
fn a_trailing_newline_does_not_add_a_line() {
    assert_eq!(counted(Language::Rust, "fn a() {}").total, 1);
    assert_eq!(counted(Language::Rust, "fn a() {}\n").total, 1);
}

#[test]
fn splits_code_comments_and_blanks() {
    let counts = counted(
        Language::Rust,
        "// what this does\nfn main() {\n\n    run();\n}\n",
    );

    assert_eq!(counts.total, 5);
    assert_eq!(counts.comment, 1);
    assert_eq!(counts.blank, 1);
    assert_eq!(counts.code, 3);
}

#[test]
fn a_trailing_comment_leaves_the_line_as_code() {
    let counts = counted(Language::Rust, "run(); // go\n");

    assert_eq!(counts.code, 1);
    assert_eq!(counts.comment, 0);
}

#[test]
fn a_doc_comment_is_a_comment() {
    let counts = counted(Language::Rust, "/// docs\n//! module docs\nfn a() {}\n");

    assert_eq!(counts.comment, 2);
    assert_eq!(counts.code, 1);
}

#[test]
fn a_block_comment_spans_its_lines() {
    let counts = counted(Language::Rust, "/*\n * hello\n */\nfn a() {}\n");

    assert_eq!(counts.comment, 3);
    assert_eq!(counts.code, 1);
}

#[test]
fn a_blank_line_inside_a_block_comment_is_a_comment() {
    let counts = counted(Language::Rust, "/*\n\n*/\n");

    assert_eq!(counts.comment, 3);
    assert_eq!(counts.blank, 0);
}

#[test]
fn a_one_line_block_comment_is_a_comment() {
    let counts = counted(Language::Rust, "/* hidden */\n");

    assert_eq!(counts.comment, 1);
    assert_eq!(counts.code, 0);
}

#[test]
fn code_after_a_closing_block_comment_is_code() {
    let counts = counted(Language::Rust, "/* note */ run();\n");

    assert_eq!(counts.code, 1);
    assert_eq!(counts.comment, 0);
}

#[test]
fn code_before_an_opening_block_comment_is_code() {
    let counts = counted(Language::Rust, "run(); /* note\nstill note */\n");

    assert_eq!(counts.code, 1);
    assert_eq!(counts.comment, 1);
}

#[test]
fn a_block_delimiter_inside_a_string_does_not_open_a_comment() {
    let counts = counted(Language::Rust, "let s = \"/*\";\nrun();\n");

    assert_eq!(counts.code, 2);
    assert_eq!(counts.comment, 0);
}

#[test]
fn an_escaped_quote_does_not_end_a_string() {
    let counts = counted(Language::Rust, "let s = \"a\\\" /* b\";\nrun();\n");

    assert_eq!(counts.code, 2);
    assert_eq!(counts.comment, 0);
}

#[test]
fn a_language_without_comment_syntax_counts_every_line_as_code() {
    let counts = counted(Language::Json, "{\n  \"a\": 1\n}\n");

    assert_eq!(counts.code, 3);
    assert_eq!(counts.comment, 0);
    assert!(!Language::Json.has_comment_syntax());
}

#[test]
fn hash_comment_languages_are_recognized() {
    let counts = counted(Language::Toml, "# why\nkey = 1\n");

    assert_eq!(counts.comment, 1);
    assert_eq!(counts.code, 1);
}

#[test]
fn sql_uses_double_dash_comments() {
    let counts = counted(Language::Sql, "-- why\nSELECT 1;\n");

    assert_eq!(counts.comment, 1);
    assert_eq!(counts.code, 1);
}

#[test]
fn html_block_comments_are_recognized() {
    let counts = counted(Language::Html, "<!-- note -->\n<p>hi</p>\n");

    assert_eq!(counts.comment, 1);
    assert_eq!(counts.code, 1);
}

#[test]
fn python_triple_quoted_blocks_are_comments() {
    let counts = counted(Language::Python, "\"\"\"\ndocs\n\"\"\"\nrun()\n");

    assert_eq!(counts.comment, 3);
    assert_eq!(counts.code, 1);
}

#[test]
fn a_line_containing_a_multi_byte_character_is_scanned_without_panicking() {
    // The scanner walks the line looking for a block-comment opener. Doing that
    // by byte index and then slicing on it splits a multi-byte character in
    // half, which is a panic rather than a wrong answer. Markdown reaches the
    // scanner because it has block comments and no line comments, so nothing
    // short-circuits ahead of it.
    let counts = counted(Language::Markdown, "A dash \u{2014} in prose\n");

    assert_eq!(counts.code, 1);
}

#[test]
fn a_multi_byte_character_before_a_block_comment_does_not_shift_the_scanner() {
    let counts = counted(Language::Rust, "let s = \"\u{2014}\"; run(); /* note */\n");

    assert_eq!(counts.code, 1);
    assert_eq!(counts.comment, 0);
}

#[test]
fn a_line_of_box_drawing_characters_is_scanned_without_panicking() {
    let counts = counted(
        Language::Markdown,
        "\u{251c}\u{2500}\u{2500} crates/\n<!-- note -->\n",
    );

    assert_eq!(counts.total, 2);
    assert_eq!(counts.code, 1);
    assert_eq!(counts.comment, 1);
}

#[test]
fn identifies_languages_from_extensions() {
    assert_eq!(Language::from_file_name("lib.rs"), Language::Rust);
    assert_eq!(Language::from_file_name("Cargo.toml"), Language::Toml);
    assert_eq!(Language::from_file_name("README.md"), Language::Markdown);
    assert_eq!(Language::from_file_name("app.TSX"), Language::TypeScript);
    assert_eq!(Language::from_file_name("main.cpp"), Language::C);
    assert_eq!(Language::from_file_name("query.sql"), Language::Sql);
    assert_eq!(Language::from_file_name("LICENSE"), Language::Other);
}

#[test]
fn every_language_has_a_label() {
    for language in [
        Language::Rust,
        Language::Toml,
        Language::Markdown,
        Language::Json,
        Language::Yaml,
        Language::Shell,
        Language::JavaScript,
        Language::TypeScript,
        Language::Html,
        Language::Css,
        Language::Python,
        Language::C,
        Language::Go,
        Language::Sql,
        Language::Other,
    ] {
        assert!(!language.label().is_empty());
    }
}

#[test]
fn the_comment_ratio_is_finite_for_a_file_with_no_code() {
    let counts = counted(Language::Rust, "// only a comment\n");

    assert!((counts.comment_ratio() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn the_comment_ratio_divides_comments_by_code() {
    let counts = counted(Language::Rust, "// a\n// b\nrun();\nstop();\n");

    assert!((counts.comment_ratio() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn counts_accumulate() {
    let mut total = LineCounts::default();
    total.add(counted(Language::Rust, "fn a() {}\n"));
    total.add(counted(Language::Rust, "// b\n\n"));

    assert_eq!(total.total, 3);
    assert_eq!(total.code, 1);
    assert_eq!(total.comment, 1);
    assert_eq!(total.blank, 1);
}
