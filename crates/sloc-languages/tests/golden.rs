/// Golden tests: known-good fixture files with expected line counts.
///
/// Each test reads a corpus file, runs the lexical analyzer, and asserts exact
/// counts. The corpus files are checked into git so regressions are caught
/// immediately on CI.
use std::path::Path;

use sloc_languages::{analyze_text, Language};

fn corpus(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read corpus file {}: {e}", path.display()))
}

// ─── C ────────────────────────────────────────────────────────────────────────

#[test]
fn c_mixed() {
    let text = corpus("c/mixed.c");
    let result = analyze_text(Language::C, &text);
    let r = &result.raw;
    // split_terminator omits the empty string after the final newline → 8 lines
    assert_eq!(r.total_physical_lines, 8, "total physical lines");
    assert_eq!(r.blank_only_lines, 1, "blank lines");
    assert_eq!(r.multi_comment_only_lines, 1, "block-comment-only lines");
    assert_eq!(r.single_comment_only_lines, 1, "single-comment-only lines");
    assert_eq!(
        r.mixed_code_single_comment_lines, 1,
        "mixed code+single-comment"
    );
    assert_eq!(
        r.mixed_code_multi_comment_lines, 1,
        "mixed code+block-comment"
    );
    assert_eq!(
        r.code_only_lines, 3,
        "pure code lines: int y=2, void foo() {{...}}, closing brace"
    );
}

// ─── Python ───────────────────────────────────────────────────────────────────

#[test]
fn python_mixed() {
    let text = corpus("python/mixed.py");
    let result = analyze_text(Language::Python, &text);
    let r = &result.raw;
    // 3 docstring lines (module doc, greet doc, Greeter class doc)
    assert_eq!(r.docstring_comment_lines, 3, "docstring lines");
    assert_eq!(r.mixed_code_single_comment_lines, 1, "mixed code+inline #");
    // blanks between sections
    assert!(r.blank_only_lines >= 2, "blank lines");
    // def greet, class Greeter, def hello, pass = 4 code lines
    assert!(r.code_only_lines >= 4, "pure code lines");
}

// ─── Rust ─────────────────────────────────────────────────────────────────────

#[test]
fn rust_mixed() {
    let text = corpus("rust/mixed.rs");
    let result = analyze_text(Language::Rust, &text);
    let r = &result.raw;
    assert_eq!(r.single_comment_only_lines, 1, "// comment-only line");
    assert_eq!(
        r.multi_comment_only_lines, 1,
        "/* block comment-only line */"
    );
    assert_eq!(
        r.mixed_code_single_comment_lines, 1,
        "mixed code + // comment"
    );
    assert!(r.code_only_lines >= 4, "pure code lines");
}

// ─── Go ───────────────────────────────────────────────────────────────────────

#[test]
fn go_mixed() {
    let text = corpus("go/mixed.go");
    let result = analyze_text(Language::Go, &text);
    let r = &result.raw;
    assert!(r.single_comment_only_lines >= 2, "// comment-only lines");
    assert_eq!(
        r.mixed_code_multi_comment_lines, 1,
        "mixed code + /* block */"
    );
    assert!(r.code_only_lines >= 3, "pure code lines");
}

// ─── TypeScript ───────────────────────────────────────────────────────────────

#[test]
fn typescript_mixed() {
    let text = corpus("typescript/mixed.ts");
    let result = analyze_text(Language::TypeScript, &text);
    let r = &result.raw;
    assert_eq!(r.single_comment_only_lines, 1, "// comment-only");
    assert_eq!(r.multi_comment_only_lines, 1, "/* block comment-only */");
    assert_eq!(
        r.mixed_code_single_comment_lines, 1,
        "mixed code + // inline"
    );
    assert!(r.code_only_lines >= 3, "pure code lines");
}

// ─── Empty file ───────────────────────────────────────────────────────────────

#[test]
fn empty_file_all_languages() {
    for lang in [
        Language::C,
        Language::Cpp,
        Language::CSharp,
        Language::Go,
        Language::Java,
        Language::JavaScript,
        Language::Python,
        Language::Rust,
        Language::Shell,
        Language::PowerShell,
        Language::TypeScript,
    ] {
        let result = analyze_text(lang, "");
        assert_eq!(
            result.raw.total_physical_lines,
            0,
            "{} should have 0 lines for empty input",
            lang.display_name()
        );
    }
}
