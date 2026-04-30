# sloc-languages

Language detection and lexical SLOC analysis for [oxide-sloc](https://github.com/oxide-sloc/oxide-sloc).

## Overview

This crate provides:

- **`Language` enum** — 41 supported languages with `display_name()`, `as_slug()`, `from_name()`
- **`analyze_text(language, text, options)`** — hand-rolled character-by-character state machine returning `RawFileAnalysis` with counts for code, comment, blank, and compiler-directive lines
- **`AnalysisOptions`** — carries IEEE 1045-1992 flags: continuation-line policy, blank-in-block-comment policy, compiler-directive tracking

Supported languages include Rust, C/C++, Python, Go, TypeScript, Java, JavaScript, Shell, SQL, Kotlin, Swift, and 30 more.

## Usage

This is an internal crate used by the oxide-sloc workspace. It is not intended for use outside this project. See the [main project](https://github.com/oxide-sloc/oxide-sloc) for documentation and releases.

```toml
# Install the tool instead:
cargo install oxide-sloc
```
