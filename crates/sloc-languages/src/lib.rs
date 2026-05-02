// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    C,
    Cpp,
    CSharp,
    Go,
    Java,
    JavaScript,
    Python,
    Rust,
    Shell,
    PowerShell,
    TypeScript,
    // --- Extended language support ---
    Assembly,
    Clojure,
    Css,
    Dart,
    Dockerfile,
    Elixir,
    Erlang,
    FSharp,
    Groovy,
    Haskell,
    Html,
    Julia,
    Kotlin,
    Lua,
    Makefile,
    Nim,
    ObjectiveC,
    Ocaml,
    Perl,
    Php,
    R,
    Ruby,
    Scala,
    Scss,
    Sql,
    Svelte,
    Swift,
    Vue,
    Xml,
    Zig,
}

impl Language {
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cpp => "C++",
            Self::CSharp => "C#",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::JavaScript => "JavaScript",
            Self::Python => "Python",
            Self::Rust => "Rust",
            Self::Shell => "Shell",
            Self::PowerShell => "PowerShell",
            Self::TypeScript => "TypeScript",
            Self::Assembly => "Assembly",
            Self::Clojure => "Clojure",
            Self::Css => "CSS",
            Self::Dart => "Dart",
            Self::Dockerfile => "Dockerfile",
            Self::Elixir => "Elixir",
            Self::Erlang => "Erlang",
            Self::FSharp => "F#",
            Self::Groovy => "Groovy",
            Self::Haskell => "Haskell",
            Self::Html => "HTML",
            Self::Julia => "Julia",
            Self::Kotlin => "Kotlin",
            Self::Lua => "Lua",
            Self::Makefile => "Makefile",
            Self::Nim => "Nim",
            Self::ObjectiveC => "Objective-C",
            Self::Ocaml => "OCaml",
            Self::Perl => "Perl",
            Self::Php => "PHP",
            Self::R => "R",
            Self::Ruby => "Ruby",
            Self::Scala => "Scala",
            Self::Scss => "SCSS",
            Self::Sql => "SQL",
            Self::Svelte => "Svelte",
            Self::Swift => "Swift",
            Self::Vue => "Vue",
            Self::Xml => "XML",
            Self::Zig => "Zig",
        }
    }

    #[must_use]
    pub const fn as_slug(&self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Go => "go",
            Self::Java => "java",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Shell => "shell",
            Self::PowerShell => "powershell",
            Self::TypeScript => "typescript",
            Self::Assembly => "assembly",
            Self::Clojure => "clojure",
            Self::Css => "css",
            Self::Dart => "dart",
            Self::Dockerfile => "dockerfile",
            Self::Elixir => "elixir",
            Self::Erlang => "erlang",
            Self::FSharp => "fsharp",
            Self::Groovy => "groovy",
            Self::Haskell => "haskell",
            Self::Html => "html",
            Self::Julia => "julia",
            Self::Kotlin => "kotlin",
            Self::Lua => "lua",
            Self::Makefile => "makefile",
            Self::Nim => "nim",
            Self::ObjectiveC => "objectivec",
            Self::Ocaml => "ocaml",
            Self::Perl => "perl",
            Self::Php => "php",
            Self::R => "r",
            Self::Ruby => "ruby",
            Self::Scala => "scala",
            Self::Scss => "scss",
            Self::Sql => "sql",
            Self::Svelte => "svelte",
            Self::Swift => "swift",
            Self::Vue => "vue",
            Self::Xml => "xml",
            Self::Zig => "zig",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "c" => Some(Self::C),
            "cpp" | "c++" | "cplusplus" => Some(Self::Cpp),
            "csharp" | "c#" | "cs" => Some(Self::CSharp),
            "go" | "golang" => Some(Self::Go),
            "java" => Some(Self::Java),
            "javascript" | "js" => Some(Self::JavaScript),
            "python" | "py" => Some(Self::Python),
            "rust" | "rs" => Some(Self::Rust),
            "shell" | "sh" | "bash" => Some(Self::Shell),
            "powershell" | "pwsh" | "ps" => Some(Self::PowerShell),
            "typescript" | "ts" => Some(Self::TypeScript),
            "assembly" | "asm" => Some(Self::Assembly),
            "clojure" | "clj" => Some(Self::Clojure),
            "css" => Some(Self::Css),
            "dart" => Some(Self::Dart),
            "dockerfile" | "docker" => Some(Self::Dockerfile),
            "elixir" | "ex" => Some(Self::Elixir),
            "erlang" | "erl" => Some(Self::Erlang),
            "fsharp" | "f#" | "fs" => Some(Self::FSharp),
            "groovy" => Some(Self::Groovy),
            "haskell" | "hs" => Some(Self::Haskell),
            "html" | "htm" => Some(Self::Html),
            "julia" | "jl" => Some(Self::Julia),
            "kotlin" | "kt" => Some(Self::Kotlin),
            "lua" => Some(Self::Lua),
            "makefile" | "make" | "mk" => Some(Self::Makefile),
            "nim" => Some(Self::Nim),
            "objectivec" | "objc" | "objective-c" => Some(Self::ObjectiveC),
            "ocaml" | "ml" => Some(Self::Ocaml),
            "perl" | "pl" => Some(Self::Perl),
            "php" => Some(Self::Php),
            "r" => Some(Self::R),
            "ruby" | "rb" => Some(Self::Ruby),
            "scala" => Some(Self::Scala),
            "scss" | "sass" => Some(Self::Scss),
            "sql" => Some(Self::Sql),
            "svelte" => Some(Self::Svelte),
            "swift" => Some(Self::Swift),
            "vue" => Some(Self::Vue),
            "xml" => Some(Self::Xml),
            "zig" => Some(Self::Zig),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawLineCounts {
    pub total_physical_lines: u64,
    pub blank_only_lines: u64,
    pub code_only_lines: u64,
    pub single_comment_only_lines: u64,
    pub multi_comment_only_lines: u64,
    pub mixed_code_single_comment_lines: u64,
    pub mixed_code_multi_comment_lines: u64,
    pub docstring_comment_lines: u64,
    pub skipped_unknown_lines: u64,
    /// Best-effort count of function/method definition lines detected lexically.
    #[serde(default)]
    pub functions: u64,
    /// Best-effort count of class/struct/trait/type definition lines detected lexically.
    #[serde(default)]
    pub classes: u64,
    /// Best-effort count of variable declaration lines detected lexically.
    #[serde(default)]
    pub variables: u64,
    /// Best-effort count of import/use/include statement lines detected lexically.
    #[serde(default)]
    pub imports: u64,
    /// Lines consisting solely of preprocessor/compiler directives (e.g. `#include`, `#define`
    /// in C/C++/Objective-C). Always a subset of `code_only_lines`. Controlled by
    /// `AnalysisConfig::count_compiler_directives`. IEEE 1045-1992 §4.2.
    #[serde(default)]
    pub compiler_directive_lines: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseMode {
    Lexical,
    LexicalBestEffort,
    TreeSitter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFileAnalysis {
    pub raw: RawLineCounts,
    pub parse_mode: ParseMode,
    pub warnings: Vec<String>,
}

/// IEEE 1045-1992 counting options passed from `sloc-core` (built from `AnalysisConfig`).
///
/// `analyze_text` accepts this struct so that the caller can control behaviour that the
/// standard defines as configurable parameters rather than fixed conventions.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisOptions {
    /// When `true` (IEEE 1045-1992 default), blank lines inside block comments count as
    /// comment lines rather than blank lines.
    pub blank_in_block_comment_as_comment: bool,
    /// When `true`, backslash-continued physical lines are collapsed into a single logical
    /// line for SLOC counting purposes (IEEE logical SLOC mode).
    pub collapse_continuation_lines: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            blank_in_block_comment_as_comment: true,
            collapse_continuation_lines: false,
        }
    }
}

#[must_use]
pub fn supported_languages() -> BTreeSet<Language> {
    [
        Language::Assembly,
        Language::C,
        Language::Clojure,
        Language::Cpp,
        Language::CSharp,
        Language::Css,
        Language::Dart,
        Language::Dockerfile,
        Language::Elixir,
        Language::Erlang,
        Language::FSharp,
        Language::Go,
        Language::Groovy,
        Language::Haskell,
        Language::Html,
        Language::Java,
        Language::JavaScript,
        Language::Julia,
        Language::Kotlin,
        Language::Lua,
        Language::Makefile,
        Language::Nim,
        Language::ObjectiveC,
        Language::Ocaml,
        Language::Perl,
        Language::Php,
        Language::PowerShell,
        Language::Python,
        Language::R,
        Language::Ruby,
        Language::Rust,
        Language::Scala,
        Language::Scss,
        Language::Shell,
        Language::Sql,
        Language::Svelte,
        Language::Swift,
        Language::TypeScript,
        Language::Vue,
        Language::Xml,
        Language::Zig,
    ]
    .into_iter()
    .collect()
}

/// Detect language from a shebang line (e.g. `#!/usr/bin/env python3`).
fn detect_by_shebang(line: &str) -> Option<Language> {
    let lower = line.to_ascii_lowercase();
    if !lower.starts_with("#!") {
        return None;
    }
    if lower.contains("python") {
        return Some(Language::Python);
    }
    if lower.contains("pwsh") || lower.contains("powershell") {
        return Some(Language::PowerShell);
    }
    if lower.contains("bash")
        || lower.contains("/sh")
        || lower.contains("zsh")
        || lower.contains("ksh")
    {
        return Some(Language::Shell);
    }
    if lower.contains("ruby") {
        return Some(Language::Ruby);
    }
    if lower.contains("perl") {
        return Some(Language::Perl);
    }
    if lower.contains("php") {
        return Some(Language::Php);
    }
    if lower.contains("node") || lower.contains("nodejs") {
        return Some(Language::JavaScript);
    }
    None
}

/// Detect language purely from a (lowercased) file extension.
fn detect_by_extension(ext: &str) -> Option<Language> {
    match ext {
        // --- Original 11 ---
        "c" | "h" => Some(Language::C),
        "cc" | "cp" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some(Language::Cpp),
        "cs" => Some(Language::CSharp),
        "go" => Some(Language::Go),
        "java" => Some(Language::Java),
        "js" | "mjs" | "cjs" => Some(Language::JavaScript),
        "py" => Some(Language::Python),
        "rs" => Some(Language::Rust),
        "sh" | "bash" | "zsh" | "ksh" => Some(Language::Shell),
        "ps1" | "psm1" | "psd1" => Some(Language::PowerShell),
        "ts" | "mts" | "cts" => Some(Language::TypeScript),
        // --- Extended 30 ---
        "asm" | "s" => Some(Language::Assembly),
        "clj" | "cljs" | "cljc" | "edn" => Some(Language::Clojure),
        "css" => Some(Language::Css),
        "dart" => Some(Language::Dart),
        "ex" | "exs" => Some(Language::Elixir),
        "erl" | "hrl" => Some(Language::Erlang),
        "fs" | "fsi" | "fsx" => Some(Language::FSharp),
        "groovy" | "gradle" => Some(Language::Groovy),
        "hs" | "lhs" => Some(Language::Haskell),
        "html" | "htm" | "xhtml" => Some(Language::Html),
        "jl" => Some(Language::Julia),
        "kt" | "kts" => Some(Language::Kotlin),
        "lua" => Some(Language::Lua),
        "mk" => Some(Language::Makefile),
        "nim" | "nims" => Some(Language::Nim),
        "m" | "mm" => Some(Language::ObjectiveC),
        "ml" | "mli" => Some(Language::Ocaml),
        "pl" | "pm" | "t" => Some(Language::Perl),
        "php" | "php3" | "php4" | "php5" | "php7" | "phtml" => Some(Language::Php),
        "r" => Some(Language::R),
        "rb" | "rake" => Some(Language::Ruby),
        "scala" | "sc" => Some(Language::Scala),
        "scss" | "sass" => Some(Language::Scss),
        "sql" => Some(Language::Sql),
        "svelte" => Some(Language::Svelte),
        "swift" => Some(Language::Swift),
        "vue" => Some(Language::Vue),
        "xml" | "xsd" | "xsl" | "xslt" | "svg" => Some(Language::Xml),
        "zig" => Some(Language::Zig),
        _ => None,
    }
}

/// Detect language from an exact filename (no extension) or well-known filename patterns.
fn detect_by_filename(filename: &str, filename_lower: &str) -> Option<Language> {
    // Dockerfile: exact name or Dockerfile.* variant
    if filename == "Dockerfile"
        || filename.starts_with("Dockerfile.")
        || filename_lower == "dockerfile"
    {
        return Some(Language::Dockerfile);
    }
    // Makefile variants
    if matches!(
        filename,
        "Makefile" | "GNUmakefile" | "makefile" | "BSDmakefile"
    ) {
        return Some(Language::Makefile);
    }
    // Ruby ecosystem files that have no extension
    if matches!(
        filename,
        "Rakefile" | "Gemfile" | "Guardfile" | "Vagrantfile" | "Fastfile" | "Podfile"
    ) {
        return Some(Language::Ruby);
    }
    None
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn detect_language(
    path: &Path,
    first_line: Option<&str>,
    extension_overrides: &BTreeMap<String, String>,
    shebang_detection: bool,
) -> Option<Language> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);

    // Extension override check (user-configured mappings win over everything)
    if let Some(ext) = extension.as_ref() {
        if let Some(override_name) = extension_overrides.get(ext.as_str()) {
            if let Some(lang) = Language::from_name(override_name) {
                return Some(lang);
            }
        }
    }

    // Filename-based detection for files that have no extension or use exact names
    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let filename_lower = filename.to_ascii_lowercase();

    if let Some(lang) = detect_by_filename(filename, &filename_lower) {
        return Some(lang);
    }

    // Extension-based detection
    if let Some(lang) = extension.as_deref().and_then(detect_by_extension) {
        return Some(lang);
    }

    // Shebang detection (last resort — only for extensionless scripts)
    if shebang_detection {
        if let Some(line) = first_line {
            if let Some(lang) = detect_by_shebang(line) {
                return Some(lang);
            }
        }
    }

    None
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn analyze_text(language: Language, text: &str, options: AnalysisOptions) -> RawFileAnalysis {
    // IEEE flags shared by all non-preprocessor languages.
    let base = IeeeFlags {
        has_preprocessor_directives: false,
        blank_in_block_comment_as_comment: options.blank_in_block_comment_as_comment,
        collapse_continuation_lines: options.collapse_continuation_lines,
    };
    // C, C++, and Objective-C have a preprocessor whose directive lines are tracked separately
    // per IEEE 1045-1992 §4.2.
    let cpp = IeeeFlags {
        has_preprocessor_directives: true,
        ..base
    };

    match language {
        Language::C => {
            #[cfg(feature = "tree-sitter")]
            if let Some(result) = ts::analyze_c(text) {
                return result;
            }
            analyze_generic(
                text,
                ScanConfig {
                    line_comments: &["//"],
                    block_comment: Some(("/*", "*/")),
                    allow_single_quote_strings: true,
                    allow_double_quote_strings: true,
                    allow_triple_quote_strings: false,
                    allow_csharp_verbatim_strings: false,
                    skip_lines: HashSet::new(),
                    symbol_patterns: SP_C,
                },
                cpp,
            )
        }
        Language::Cpp => {
            // tree-sitter-c also parses C++ with acceptable accuracy for SLOC counting.
            #[cfg(feature = "tree-sitter")]
            if let Some(result) = ts::analyze_c(text) {
                return result;
            }
            analyze_generic(
                text,
                ScanConfig {
                    line_comments: &["//"],
                    block_comment: Some(("/*", "*/")),
                    allow_single_quote_strings: true,
                    allow_double_quote_strings: true,
                    allow_triple_quote_strings: false,
                    allow_csharp_verbatim_strings: false,
                    skip_lines: HashSet::new(),
                    symbol_patterns: SP_CPP,
                },
                cpp,
            )
        }
        Language::CSharp => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: true,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_CSHARP,
            },
            base,
        ),
        Language::Go => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_GO,
            },
            base,
        ),
        Language::Java => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_JAVA,
            },
            base,
        ),
        Language::JavaScript | Language::Svelte | Language::Vue => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_JS,
            },
            base,
        ),
        Language::Rust => analyze_generic(
            text,
            ScanConfig {
                // Rust also has //! and /// doc comments — they parse the same as //
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: false,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_RUST,
            },
            base,
        ),
        Language::Shell => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: None,
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_SHELL,
            },
            base,
        ),
        Language::PowerShell => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: Some(("<#", "#>")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_POWERSHELL,
            },
            base,
        ),
        Language::TypeScript => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_TS,
            },
            base,
        ),
        Language::Python => {
            #[cfg(feature = "tree-sitter")]
            if let Some(result) = ts::analyze_python(text) {
                return result;
            }
            let docstring_lines = detect_python_docstring_lines(text);
            analyze_generic(
                text,
                ScanConfig {
                    line_comments: &["#"],
                    block_comment: None,
                    allow_single_quote_strings: true,
                    allow_double_quote_strings: true,
                    allow_triple_quote_strings: true,
                    allow_csharp_verbatim_strings: false,
                    skip_lines: docstring_lines,
                    symbol_patterns: SP_PYTHON,
                },
                base,
            )
        }
        // --- Extended language analyzers ---
        Language::Assembly => analyze_generic(
            text,
            ScanConfig {
                line_comments: &[";"],
                block_comment: None,
                allow_single_quote_strings: false,
                allow_double_quote_strings: false,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_ASSEMBLY,
            },
            base,
        ),
        Language::Clojure => analyze_generic(
            text,
            ScanConfig {
                line_comments: &[";"],
                block_comment: None,
                allow_single_quote_strings: false,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_CLOJURE,
            },
            base,
        ),
        Language::Css => analyze_generic(
            text,
            ScanConfig {
                line_comments: &[],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_NONE,
            },
            base,
        ),
        Language::Dart => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_DART,
            },
            base,
        ),
        Language::Dockerfile | Language::Makefile => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: None,
                allow_single_quote_strings: false,
                allow_double_quote_strings: false,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_NONE,
            },
            base,
        ),
        Language::Elixir => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: None,
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_ELIXIR,
            },
            base,
        ),
        Language::Erlang => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["%"],
                block_comment: None,
                allow_single_quote_strings: false,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_ERLANG,
            },
            base,
        ),
        Language::FSharp => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("(*", "*)")),
                allow_single_quote_strings: false,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_FSHARP,
            },
            base,
        ),
        Language::Groovy => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_GROOVY,
            },
            base,
        ),
        Language::Haskell => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["--"],
                block_comment: Some(("{-", "-}")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_HASKELL,
            },
            base,
        ),
        Language::Html | Language::Xml => analyze_generic(
            text,
            ScanConfig {
                line_comments: &[],
                block_comment: Some(("<!--", "-->")),
                allow_single_quote_strings: false,
                allow_double_quote_strings: false,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_NONE,
            },
            base,
        ),
        Language::Julia => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: Some(("#=", "=#")),
                allow_single_quote_strings: false,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: true,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_JULIA,
            },
            base,
        ),
        Language::Kotlin => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_KOTLIN,
            },
            base,
        ),
        Language::Lua => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["--"],
                block_comment: Some(("--[[", "]]")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_LUA,
            },
            base,
        ),
        Language::Nim => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: Some(("#[", "]#")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_NIM,
            },
            base,
        ),
        Language::ObjectiveC => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_OBJECTIVEC,
            },
            cpp,
        ),
        Language::Ocaml => analyze_generic(
            text,
            ScanConfig {
                line_comments: &[],
                block_comment: Some(("(*", "*)")),
                allow_single_quote_strings: false,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_OCAML,
            },
            base,
        ),
        Language::Perl => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: None,
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_PERL,
            },
            base,
        ),
        Language::Php => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//", "#"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_PHP,
            },
            base,
        ),
        Language::R => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: None,
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_R,
            },
            base,
        ),
        Language::Ruby => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["#"],
                block_comment: None,
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_RUBY,
            },
            base,
        ),
        Language::Scala => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_SCALA,
            },
            base,
        ),
        Language::Scss => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_NONE,
            },
            base,
        ),
        Language::Sql => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["--"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: true,
                allow_double_quote_strings: false,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_SQL,
            },
            base,
        ),
        Language::Swift => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                allow_single_quote_strings: false,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_SWIFT,
            },
            base,
        ),
        Language::Zig => analyze_generic(
            text,
            ScanConfig {
                line_comments: &["//"],
                block_comment: None,
                allow_single_quote_strings: true,
                allow_double_quote_strings: true,
                allow_triple_quote_strings: false,
                allow_csharp_verbatim_strings: false,
                skip_lines: HashSet::new(),
                symbol_patterns: SP_ZIG,
            },
            base,
        ),
    }
}

/// Per-language keyword prefixes used for best-effort structural symbol detection.
/// Each slice lists line prefixes (after leading whitespace is stripped) that indicate
/// a definition of that category. Empty slice = detection disabled for that category.
#[derive(Debug, Clone, Copy)]
struct SymbolPatterns {
    functions: &'static [&'static str],
    classes: &'static [&'static str],
    variables: &'static [&'static str],
    imports: &'static [&'static str],
}

impl SymbolPatterns {
    const fn none() -> Self {
        Self {
            functions: &[],
            classes: &[],
            variables: &[],
            imports: &[],
        }
    }
}

const SP_NONE: SymbolPatterns = SymbolPatterns::none();

const SP_RUST: SymbolPatterns = SymbolPatterns {
    functions: &[
        "fn ",
        "pub fn ",
        "pub(crate) fn ",
        "pub(super) fn ",
        "async fn ",
        "pub async fn ",
        "pub(crate) async fn ",
        "unsafe fn ",
        "pub unsafe fn ",
        "pub(crate) unsafe fn ",
        "const fn ",
        "pub const fn ",
        "pub(crate) const fn ",
        "extern fn ",
        "pub extern fn ",
    ],
    classes: &[
        "struct ",
        "pub struct ",
        "pub(crate) struct ",
        "enum ",
        "pub enum ",
        "pub(crate) enum ",
        "trait ",
        "pub trait ",
        "pub(crate) trait ",
        "impl ",
        "impl<",
        "type ",
        "pub type ",
        "pub(crate) type ",
    ],
    variables: &["let ", "let mut "],
    imports: &["use ", "pub use ", "pub(crate) use ", "extern crate "],
};

const SP_PYTHON: SymbolPatterns = SymbolPatterns {
    functions: &["def ", "async def "],
    classes: &["class "],
    variables: &[],
    imports: &["import ", "from "],
};

const SP_JS: SymbolPatterns = SymbolPatterns {
    functions: &[
        "function ",
        "async function ",
        "export function ",
        "export async function ",
        "export default function ",
    ],
    classes: &["class ", "export class ", "export default class "],
    variables: &[
        "var ",
        "let ",
        "const ",
        "export var ",
        "export let ",
        "export const ",
    ],
    imports: &["import "],
};

const SP_TS: SymbolPatterns = SymbolPatterns {
    functions: &[
        "function ",
        "async function ",
        "export function ",
        "export async function ",
        "export default function ",
    ],
    classes: &[
        "class ",
        "export class ",
        "export default class ",
        "abstract class ",
        "export abstract class ",
        "interface ",
        "export interface ",
        "declare class ",
        "declare interface ",
    ],
    variables: &[
        "var ",
        "let ",
        "const ",
        "export var ",
        "export let ",
        "export const ",
    ],
    imports: &["import "],
};

const SP_GO: SymbolPatterns = SymbolPatterns {
    functions: &["func "],
    classes: &["type "],
    variables: &["var "],
    imports: &["import "],
};

const SP_JAVA: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &[
        "class ",
        "public class ",
        "private class ",
        "protected class ",
        "abstract class ",
        "final class ",
        "public abstract class ",
        "public final class ",
        "interface ",
        "public interface ",
        "enum ",
        "public enum ",
        "record ",
        "public record ",
        "@interface ",
    ],
    variables: &[],
    imports: &["import "],
};

const SP_CSHARP: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &[
        "class ",
        "public class ",
        "private class ",
        "protected class ",
        "internal class ",
        "abstract class ",
        "sealed class ",
        "static class ",
        "partial class ",
        "public abstract class ",
        "public sealed class ",
        "public static class ",
        "interface ",
        "public interface ",
        "internal interface ",
        "enum ",
        "public enum ",
        "struct ",
        "public struct ",
        "record ",
        "public record ",
    ],
    variables: &["var "],
    imports: &["using "],
};

const SP_C: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &[
        "struct ",
        "typedef struct ",
        "union ",
        "typedef union ",
        "typedef enum ",
    ],
    variables: &[],
    imports: &["#include "],
};

const SP_CPP: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &["class ", "struct ", "namespace ", "template "],
    variables: &[],
    imports: &["#include "],
};

const SP_SHELL: SymbolPatterns = SymbolPatterns {
    functions: &["function "],
    classes: &[],
    variables: &["declare ", "local ", "export "],
    imports: &["source ", ". "],
};

const SP_POWERSHELL: SymbolPatterns = SymbolPatterns {
    functions: &["function ", "Function "],
    classes: &["class "],
    variables: &[],
    imports: &["Import-Module ", "using "],
};

const SP_KOTLIN: SymbolPatterns = SymbolPatterns {
    functions: &[
        "fun ",
        "private fun ",
        "public fun ",
        "protected fun ",
        "internal fun ",
        "override fun ",
        "suspend fun ",
        "abstract fun ",
        "open fun ",
        "private suspend fun ",
        "public suspend fun ",
    ],
    classes: &[
        "class ",
        "data class ",
        "sealed class ",
        "abstract class ",
        "open class ",
        "object ",
        "companion object",
        "interface ",
        "enum class ",
        "annotation class ",
    ],
    variables: &["val ", "var ", "private val ", "private var ", "const val "],
    imports: &["import "],
};

const SP_SWIFT: SymbolPatterns = SymbolPatterns {
    functions: &[
        "func ",
        "private func ",
        "public func ",
        "internal func ",
        "override func ",
        "open func ",
        "static func ",
        "class func ",
        "mutating func ",
        "private static func ",
        "public static func ",
    ],
    classes: &[
        "class ",
        "struct ",
        "protocol ",
        "enum ",
        "extension ",
        "actor ",
        "public class ",
        "private class ",
        "open class ",
        "final class ",
        "public struct ",
        "private struct ",
        "public protocol ",
    ],
    variables: &[
        "var ",
        "let ",
        "private var ",
        "private let ",
        "static var ",
        "static let ",
    ],
    imports: &["import "],
};

const SP_RUBY: SymbolPatterns = SymbolPatterns {
    functions: &["def ", "private def ", "protected def "],
    classes: &["class ", "module "],
    variables: &[],
    imports: &["require ", "require_relative "],
};

const SP_SCALA: SymbolPatterns = SymbolPatterns {
    functions: &["def ", "private def ", "protected def ", "override def "],
    classes: &[
        "class ",
        "case class ",
        "abstract class ",
        "sealed class ",
        "object ",
        "trait ",
    ],
    variables: &["val ", "var ", "lazy val "],
    imports: &["import "],
};

const SP_PHP: SymbolPatterns = SymbolPatterns {
    functions: &[
        "function ",
        "public function ",
        "private function ",
        "protected function ",
        "static function ",
        "abstract function ",
        "final function ",
        "public static function ",
        "private static function ",
        "protected static function ",
    ],
    classes: &[
        "class ",
        "abstract class ",
        "final class ",
        "interface ",
        "trait ",
        "enum ",
    ],
    variables: &[],
    imports: &[
        "use ",
        "require ",
        "require_once ",
        "include ",
        "include_once ",
    ],
};

const SP_ELIXIR: SymbolPatterns = SymbolPatterns {
    functions: &[
        "def ",
        "defp ",
        "defmacro ",
        "defmacrop ",
        "defguard ",
        "defguardp ",
    ],
    classes: &["defmodule ", "defprotocol ", "defimpl "],
    variables: &[],
    imports: &["import ", "alias ", "use ", "require "],
};

const SP_ERLANG: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &["-module("],
    variables: &[],
    imports: &["-import(", "-include(", "-include_lib("],
};

const SP_FSHARP: SymbolPatterns = SymbolPatterns {
    functions: &[
        "let ",
        "let rec ",
        "member ",
        "override ",
        "abstract member ",
    ],
    classes: &["type "],
    variables: &["let mutable "],
    imports: &["open "],
};

const SP_GROOVY: SymbolPatterns = SymbolPatterns {
    functions: &["def ", "private def ", "public def ", "protected def "],
    classes: &["class ", "abstract class ", "interface ", "enum ", "trait "],
    variables: &[],
    imports: &["import "],
};

const SP_HASKELL: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &["class ", "data ", "newtype ", "type "],
    variables: &[],
    imports: &["import "],
};

const SP_LUA: SymbolPatterns = SymbolPatterns {
    functions: &["function ", "local function "],
    classes: &[],
    variables: &["local "],
    imports: &[],
};

const SP_NIM: SymbolPatterns = SymbolPatterns {
    functions: &[
        "proc ",
        "func ",
        "method ",
        "iterator ",
        "converter ",
        "template ",
        "macro ",
    ],
    classes: &["type "],
    variables: &["var ", "let ", "const "],
    imports: &["import ", "from "],
};

const SP_OBJECTIVEC: SymbolPatterns = SymbolPatterns {
    functions: &["- (", "+ ("],
    classes: &["@interface ", "@implementation ", "@protocol "],
    variables: &[],
    imports: &["#import ", "#include "],
};

const SP_OCAML: SymbolPatterns = SymbolPatterns {
    functions: &["let ", "let rec "],
    classes: &["type ", "module ", "class "],
    variables: &[],
    imports: &["open "],
};

const SP_PERL: SymbolPatterns = SymbolPatterns {
    functions: &["sub "],
    classes: &["package "],
    variables: &["my ", "our ", "local "],
    imports: &["use ", "require "],
};

const SP_CLOJURE: SymbolPatterns = SymbolPatterns {
    functions: &["(defn ", "(defn- ", "(defmacro ", "(defmulti "],
    classes: &[
        "(defrecord ",
        "(defprotocol ",
        "(deftype ",
        "(definterface ",
    ],
    variables: &["(def ", "(defonce "],
    imports: &["(ns ", "(require "],
};

const SP_JULIA: SymbolPatterns = SymbolPatterns {
    functions: &["function ", "macro "],
    classes: &[
        "struct ",
        "mutable struct ",
        "abstract type ",
        "primitive type ",
    ],
    variables: &["const "],
    imports: &["import ", "using "],
};

const SP_DART: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &["class ", "abstract class ", "mixin ", "extension ", "enum "],
    variables: &["var ", "final ", "const ", "late "],
    imports: &["import "],
};

const SP_R: SymbolPatterns = SymbolPatterns {
    functions: &[],
    classes: &[],
    variables: &[],
    imports: &["library(", "source("],
};

const SP_SQL: SymbolPatterns = SymbolPatterns {
    functions: &[
        "create function ",
        "create or replace function ",
        "create procedure ",
        "create or replace procedure ",
        "CREATE FUNCTION ",
        "CREATE OR REPLACE FUNCTION ",
        "CREATE PROCEDURE ",
        "CREATE OR REPLACE PROCEDURE ",
    ],
    classes: &[
        "create table ",
        "create view ",
        "create schema ",
        "CREATE TABLE ",
        "CREATE VIEW ",
        "CREATE SCHEMA ",
    ],
    variables: &["declare ", "DECLARE "],
    imports: &[],
};

const SP_ASSEMBLY: SymbolPatterns = SymbolPatterns {
    functions: &["proc ", "PROC "],
    classes: &[],
    variables: &[],
    imports: &["include ", "INCLUDE ", "%include "],
};

const SP_ZIG: SymbolPatterns = SymbolPatterns {
    functions: &[
        "fn ",
        "pub fn ",
        "export fn ",
        "inline fn ",
        "pub inline fn ",
    ],
    classes: &[],
    variables: &["var ", "pub var "],
    imports: &[],
};

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
struct ScanConfig {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    allow_single_quote_strings: bool,
    allow_double_quote_strings: bool,
    allow_triple_quote_strings: bool,
    allow_csharp_verbatim_strings: bool,
    skip_lines: HashSet<usize>,
    symbol_patterns: SymbolPatterns,
}

/// Per-call IEEE 1045-1992 flags derived from `AnalysisOptions` plus per-language properties.
/// Private to this crate; constructed inside `analyze_text`.
#[derive(Debug, Clone, Copy)]
struct IeeeFlags {
    /// True for C, C++, and Objective-C — languages with a C preprocessor.
    has_preprocessor_directives: bool,
    /// Mirrors `AnalysisOptions::blank_in_block_comment_as_comment`.
    blank_in_block_comment_as_comment: bool,
    /// Mirrors `AnalysisOptions::collapse_continuation_lines`.
    collapse_continuation_lines: bool,
}

#[derive(Debug, Clone, Copy)]
enum StringState {
    Single(char),
    Triple(&'static str),
    VerbatimDouble,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
struct LineFacts {
    has_code: bool,
    has_single_comment: bool,
    has_multi_comment: bool,
    has_docstring: bool,
}

/// Process one character while the lexer is inside a string literal.
///
/// Returns `(new_string_state, advance)` where `advance` is the number of chars to skip.
fn process_string_char(
    state: StringState,
    chars: &[char],
    i: usize,
) -> (Option<StringState>, usize) {
    match state {
        StringState::Single(delim) => {
            if chars[i] == '\\' {
                return (Some(state), 2); // skip escaped character
            }
            if chars[i] == delim {
                (None, 1)
            } else {
                (Some(state), 1)
            }
        }
        StringState::Triple(delim) => {
            if starts_with(chars, i, delim) {
                (None, delim.len())
            } else {
                (Some(state), 1)
            }
        }
        StringState::VerbatimDouble => {
            if starts_with(chars, i, "\"\"") {
                return (Some(state), 2); // escaped quote-quote inside verbatim string
            }
            if chars[i] == '"' {
                (None, 1)
            } else {
                (Some(state), 1)
            }
        }
    }
}

/// Process one character while the lexer is inside a block comment.
///
/// Returns `(still_in_block_comment, advance)`.
fn process_block_comment_char(chars: &[char], i: usize, close: &str) -> (bool, usize) {
    if starts_with(chars, i, close) {
        (false, close.len())
    } else {
        (true, 1)
    }
}

/// Attempt to begin a new string literal at position `i`.
///
/// Returns `Some((new_state, advance))` when a string opener is detected, else `None`.
fn try_open_string(chars: &[char], i: usize, config: &ScanConfig) -> Option<(StringState, usize)> {
    if config.allow_csharp_verbatim_strings && starts_with(chars, i, "@\"") {
        return Some((StringState::VerbatimDouble, 2));
    }
    if config.allow_triple_quote_strings {
        if starts_with(chars, i, "\"\"\"") {
            return Some((StringState::Triple("\"\"\""), 3));
        }
        if starts_with(chars, i, "'''") {
            return Some((StringState::Triple("'''"), 3));
        }
    }
    if config.allow_single_quote_strings && chars[i] == '\'' {
        return Some((StringState::Single('\''), 1));
    }
    if config.allow_double_quote_strings && chars[i] == '"' {
        return Some((StringState::Single('"'), 1));
    }
    None
}

/// Advance past one character position while inside a block comment.
///
/// Updates `in_block_comment` if the closing delimiter is found and returns the
/// number of characters consumed. Returns 0 when no block-comment config is set
/// (preserving the caller's `continue`-without-advance behaviour for that impossible state).
fn step_through_block_comment(
    chars: &[char],
    i: usize,
    block_comment: Option<(&'static str, &'static str)>,
    in_block_comment: &mut bool,
) -> usize {
    if let Some((_, close)) = block_comment {
        let (still_in, advance) = process_block_comment_char(chars, i, close);
        *in_block_comment = still_in;
        return advance;
    }
    0
}

/// If the character at `i` starts a block comment, return the length of the opening
/// delimiter so the caller can advance past it. Returns `None` if no match.
fn try_open_block_comment(
    chars: &[char],
    i: usize,
    block_comment: Option<(&'static str, &'static str)>,
) -> Option<usize> {
    let (open, _) = block_comment?;
    starts_with(chars, i, open).then_some(open.len())
}

/// Scan a single physical line and update `facts`, `in_block_comment`, and `string_state`.
///
/// Returns `true` when the caller should break out of the per-line loop early (line comment hit).
fn scan_line(
    chars: &[char],
    config: &ScanConfig,
    facts: &mut LineFacts,
    in_block_comment: &mut bool,
    string_state: &mut Option<StringState>,
) {
    let mut i = 0usize;
    while i < chars.len() {
        // Inside a string literal — advance until the closing delimiter.
        if let Some(state) = *string_state {
            facts.has_code = true;
            let (new_state, advance) = process_string_char(state, chars, i);
            *string_state = new_state;
            i += advance;
            continue;
        }

        // Inside a block comment — advance until the closing delimiter.
        if *in_block_comment {
            facts.has_multi_comment = true;
            i += step_through_block_comment(chars, i, config.block_comment, in_block_comment);
            continue;
        }

        // Whitespace outside any string/comment — skip.
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }

        // Attempt to open a string literal.
        if let Some((new_state, advance)) = try_open_string(chars, i, config) {
            facts.has_code = true;
            *string_state = Some(new_state);
            i += advance;
            continue;
        }

        // Attempt to open a block comment.
        if let Some(advance) = try_open_block_comment(chars, i, config.block_comment) {
            facts.has_multi_comment = true;
            *in_block_comment = true;
            i += advance;
            continue;
        }

        // Line comment — rest of the line is a comment; stop scanning.
        if config
            .line_comments
            .iter()
            .any(|prefix| starts_with(chars, i, prefix))
        {
            facts.has_single_comment = true;
            break;
        }

        // Plain code character.
        facts.has_code = true;
        i += 1;
    }
}

/// Apply IEEE 1045-1992 §4.2 preprocessor-directive tracking and continuation-line merging,
/// then emit the finalized `LineFacts` for this physical line.
///
/// Returns `None` when the line is part of a continuation sequence and should be deferred.
fn finalize_line_facts(
    facts: LineFacts,
    trimmed: &str,
    raw: &mut RawLineCounts,
    ieee: IeeeFlags,
    in_block_comment: bool,
    string_state: Option<StringState>,
    pending_continuation: &mut Option<LineFacts>,
) -> Option<LineFacts> {
    // IEEE 1045-1992 §4.2: track preprocessor/compiler directive lines (C/C++/ObjC).
    // A directive line is a pure code line (no comment on the same physical line) whose
    // trimmed content starts with '#'.
    if ieee.has_preprocessor_directives
        && facts.has_code
        && !facts.has_single_comment
        && !facts.has_multi_comment
        && trimmed.starts_with('#')
    {
        raw.compiler_directive_lines += 1;
    }

    // IEEE 1045-1992 continuation-line handling.
    // A line is a continuation starter when it ends with '\' outside any comment or string.
    let is_continuation = ieee.collapse_continuation_lines
        && !in_block_comment
        && string_state.is_none()
        && trimmed.ends_with('\\');

    if is_continuation {
        let pending = pending_continuation.get_or_insert_with(LineFacts::default);
        pending.has_code |= facts.has_code;
        pending.has_single_comment |= facts.has_single_comment;
        pending.has_multi_comment |= facts.has_multi_comment;
        pending.has_docstring |= facts.has_docstring;
        return None; // defer classification until the sequence ends
    }

    // Merge any accumulated continuation facts into the final line.
    let emit = if let Some(pending) = pending_continuation.take() {
        LineFacts {
            has_code: pending.has_code | facts.has_code,
            has_single_comment: pending.has_single_comment | facts.has_single_comment,
            has_multi_comment: pending.has_multi_comment | facts.has_multi_comment,
            has_docstring: pending.has_docstring | facts.has_docstring,
        }
    } else {
        facts
    };
    Some(emit)
}

/// Scan and classify one physical line, updating all running state in place.
///
/// Pre-classified lines (present in `config.skip_lines`) are counted as docstring-comment
/// lines and returned early without further analysis.
#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::too_many_arguments)]
fn process_physical_line(
    line: &str,
    line_idx: usize,
    config: &ScanConfig,
    raw: &mut RawLineCounts,
    in_block_comment: &mut bool,
    string_state: &mut Option<StringState>,
    pending_continuation: &mut Option<LineFacts>,
    ieee: IeeeFlags,
) {
    raw.total_physical_lines += 1;

    if config.skip_lines.contains(&line_idx) {
        raw.docstring_comment_lines += 1;
        return;
    }

    let trimmed = line.trim();
    let mut facts = LineFacts::default();

    // IEEE 1045-1992: blank lines inside block comments are comment lines by default.
    // When blank_in_block_comment_as_comment is false, blank lines keep their blank
    // classification even while inside a block comment.
    if *in_block_comment && (ieee.blank_in_block_comment_as_comment || !trimmed.is_empty()) {
        facts.has_multi_comment = true;
    }

    let chars: Vec<char> = line.chars().collect();
    scan_line(&chars, config, &mut facts, in_block_comment, string_state);

    let Some(emit) = finalize_line_facts(
        facts,
        trimmed,
        raw,
        ieee,
        *in_block_comment,
        *string_state,
        pending_continuation,
    ) else {
        return;
    };

    classify_line(raw, &emit, trimmed);

    if emit.has_code {
        let (f, c, v, i) = count_symbols(&config.symbol_patterns, trimmed);
        raw.functions += f;
        raw.classes += c;
        raw.variables += v;
        raw.imports += i;
    }
}

#[allow(clippy::needless_pass_by_value)]
fn analyze_generic(text: &str, config: ScanConfig, ieee: IeeeFlags) -> RawFileAnalysis {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split_terminator('\n').collect();

    let mut raw = RawLineCounts::default();
    let mut warnings = Vec::new();

    let mut in_block_comment = false;
    let mut string_state: Option<StringState> = None;
    // IEEE continuation-line state: accumulates facts across a backslash-continued sequence.
    let mut pending_continuation: Option<LineFacts> = None;

    for (line_idx, line) in lines.iter().enumerate() {
        process_physical_line(
            line,
            line_idx,
            &config,
            &mut raw,
            &mut in_block_comment,
            &mut string_state,
            &mut pending_continuation,
            ieee,
        );
    }

    // Flush any pending continuation that reaches end-of-file without a closing line.
    if let Some(pending) = pending_continuation.take() {
        classify_line(&mut raw, &pending, "");
    }

    if in_block_comment {
        warnings.push("unclosed block comment detected; result is best effort".into());
    }
    if string_state.is_some() {
        warnings.push("unclosed string literal detected; result is best effort".into());
    }

    RawFileAnalysis {
        raw,
        parse_mode: if warnings.is_empty() {
            ParseMode::Lexical
        } else {
            ParseMode::LexicalBestEffort
        },
        warnings,
    }
}

const fn classify_line(raw: &mut RawLineCounts, facts: &LineFacts, trimmed: &str) {
    if facts.has_docstring {
        raw.docstring_comment_lines += 1;
    } else if !facts.has_code
        && !facts.has_single_comment
        && !facts.has_multi_comment
        && trimmed.is_empty()
    {
        raw.blank_only_lines += 1;
    } else if facts.has_code && facts.has_single_comment {
        raw.mixed_code_single_comment_lines += 1;
    } else if facts.has_code && facts.has_multi_comment {
        raw.mixed_code_multi_comment_lines += 1;
    } else if facts.has_code {
        raw.code_only_lines += 1;
    } else if facts.has_single_comment {
        raw.single_comment_only_lines += 1;
    } else if facts.has_multi_comment {
        raw.multi_comment_only_lines += 1;
    } else if trimmed.is_empty() {
        raw.blank_only_lines += 1;
    } else {
        raw.skipped_unknown_lines += 1;
    }
}

fn count_symbols(patterns: &SymbolPatterns, trimmed: &str) -> (u64, u64, u64, u64) {
    let hit = |pats: &[&str]| u64::from(pats.iter().any(|p| trimmed.starts_with(p)));
    (
        hit(patterns.functions),
        hit(patterns.classes),
        hit(patterns.variables),
        hit(patterns.imports),
    )
}

fn starts_with(chars: &[char], index: usize, needle: &str) -> bool {
    let needle_chars: Vec<char> = needle.chars().collect();
    chars.get(index..index + needle_chars.len()) == Some(needle_chars.as_slice())
}

#[derive(Debug, Clone)]
struct PyContext {
    indent: usize,
    expect_docstring: bool,
}

/// Update `contexts` to pop any scopes that the current `indent` has outdented past.
fn py_pop_outdented_contexts(contexts: &mut Vec<PyContext>, indent: usize) {
    while contexts.len() > 1 && indent < contexts.last().map_or(0, |c| c.indent) {
        contexts.pop();
    }
}

/// Handle `pending_block_indent` transition: push a new docstring-expecting context when we
/// detect the first indented line of a new block, or cancel the pending state otherwise.
fn py_handle_pending_indent(
    pending_block_indent: &mut Option<usize>,
    contexts: &mut Vec<PyContext>,
    indent: usize,
    trimmed: &str,
) {
    let Some(base_indent) = *pending_block_indent else {
        return;
    };
    if indent > base_indent {
        contexts.push(PyContext {
            indent,
            expect_docstring: true,
        });
        *pending_block_indent = None;
    } else if !trimmed.starts_with('@') {
        *pending_block_indent = None;
    }
}

/// Check whether the current line is a docstring opener in the current context.
///
/// If it is, records the line, adjusts `ctx.expect_docstring`, and optionally sets
/// `active_docstring` for multi-line docstrings. Returns `true` when the caller should
/// `continue` to the next line.
fn py_try_record_docstring(
    ctx: &mut PyContext,
    trimmed: &str,
    idx: usize,
    docstring_lines: &mut HashSet<usize>,
    active_docstring: &mut Option<(&'static str, usize)>,
) -> bool {
    if !ctx.expect_docstring {
        return false;
    }
    if let Some(delim) = docstring_delimiter(trimmed) {
        docstring_lines.insert(idx);
        ctx.expect_docstring = false;
        if !closes_triple_docstring(trimmed, delim, true) {
            *active_docstring = Some((delim, idx));
        }
        return true;
    }
    ctx.expect_docstring = false;
    false
}

/// Advance through an active multi-line docstring: marks the current line and clears
/// `active_docstring` when the closing delimiter is found. Returns `true` when the caller
/// should `continue` to the next line (i.e. we were inside a docstring).
fn track_active_docstring(
    active_docstring: &mut Option<(&'static str, usize)>,
    docstring_lines: &mut HashSet<usize>,
    idx: usize,
    trimmed: &str,
) -> bool {
    let Some((delim, start_line)) = *active_docstring else {
        return false;
    };
    docstring_lines.insert(idx);
    if closes_triple_docstring(trimmed, delim, idx == start_line) {
        *active_docstring = None;
    }
    true
}

/// Attempt to record a docstring opener using the top of the context stack.
/// Returns `true` when the caller should `continue` to the next line.
fn try_record_docstring_if_context(
    contexts: &mut [PyContext],
    trimmed: &str,
    idx: usize,
    docstring_lines: &mut HashSet<usize>,
    active_docstring: &mut Option<(&'static str, usize)>,
) -> bool {
    let Some(ctx) = contexts.last_mut() else {
        return false;
    };
    py_try_record_docstring(ctx, trimmed, idx, docstring_lines, active_docstring)
}

/// If an unclosed docstring is still active at end-of-file, mark all remaining lines.
fn mark_unclosed_docstring_lines(
    active_docstring: Option<&(&'static str, usize)>,
    docstring_lines: &mut HashSet<usize>,
    num_lines: usize,
) {
    if let Some(&(_, start_line)) = active_docstring {
        for idx in start_line..num_lines {
            docstring_lines.insert(idx);
        }
    }
}

fn detect_python_docstring_lines(text: &str) -> HashSet<usize> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split_terminator('\n').collect();

    let mut docstring_lines = HashSet::new();
    let mut contexts = vec![PyContext {
        indent: 0,
        expect_docstring: true,
    }];
    let mut pending_block_indent: Option<usize> = None;
    let mut active_docstring: Option<(&'static str, usize)> = None;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let indent = leading_indent(line);

        if track_active_docstring(&mut active_docstring, &mut docstring_lines, idx, trimmed) {
            continue;
        }

        // Blank lines and comment lines don't affect docstring detection.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        py_pop_outdented_contexts(&mut contexts, indent);
        py_handle_pending_indent(&mut pending_block_indent, &mut contexts, indent, trimmed);

        if try_record_docstring_if_context(
            &mut contexts,
            trimmed,
            idx,
            &mut docstring_lines,
            &mut active_docstring,
        ) {
            continue;
        }

        if is_python_block_header(trimmed) {
            pending_block_indent = Some(indent);
        }
    }

    mark_unclosed_docstring_lines(active_docstring.as_ref(), &mut docstring_lines, lines.len());

    docstring_lines
}

fn leading_indent(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

fn is_python_block_header(trimmed: &str) -> bool {
    (trimmed.starts_with("def ")
        || trimmed.starts_with("async def ")
        || trimmed.starts_with("class "))
        && trimmed.ends_with(':')
}

fn docstring_delimiter(trimmed: &str) -> Option<&'static str> {
    let mut idx = 0usize;
    let bytes = trimmed.as_bytes();
    while idx < bytes.len() {
        let c = bytes[idx] as char;
        if matches!(c, 'r' | 'R' | 'u' | 'U' | 'b' | 'B' | 'f' | 'F') {
            idx += 1;
            continue;
        }
        break;
    }

    let rest = &trimmed[idx..];
    if rest.starts_with("\"\"\"") {
        Some("\"\"\"")
    } else if rest.starts_with("'''") {
        Some("'''")
    } else {
        None
    }
}

fn closes_triple_docstring(trimmed: &str, delim: &str, same_line_as_start: bool) -> bool {
    let mut occurrences = 0usize;
    let mut search = trimmed;
    while let Some(index) = search.find(delim) {
        occurrences += 1;
        search = &search[index + delim.len()..];
    }

    if same_line_as_start {
        occurrences >= 2
    } else {
        occurrences >= 1
    }
}

/// Tree-sitter-backed adapters (compiled only when the `tree-sitter` feature is enabled).
///
/// When parsing succeeds the result is used directly; on any failure the caller falls back
/// to the lexical state machine.
#[cfg(feature = "tree-sitter")]
pub mod ts {
    use tree_sitter::Node;

    use super::{ParseMode, RawFileAnalysis, RawLineCounts};

    /// Classify every line of `text` using a tree-sitter grammar.
    ///
    /// `comment_node_kinds` — node type names that represent comments in this grammar
    /// `docstring_stmt_kind` — optional parent node type whose direct `string` child is a docstring
    fn analyze_lines(
        text: &str,
        ts_language: &tree_sitter::Language,
        comment_node_kinds: &[&str],
        docstring_stmt_kind: Option<&str>,
    ) -> Option<RawFileAnalysis> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(ts_language).ok()?;
        let tree = parser.parse(text, None)?;

        let lines: Vec<&str> = text.split_terminator('\n').collect();
        let n = lines.len();

        let mut has_code = vec![false; n];
        let mut has_comment = vec![false; n];
        let mut comment_is_block = vec![false; n];
        let mut has_docstring = vec![false; n];

        // Walk every node in the tree and mark line arrays.
        let mut ctx = VisitCtx {
            source: text.as_bytes(),
            comment_kinds: comment_node_kinds,
            docstring_stmt_kind,
            has_code: &mut has_code,
            has_comment: &mut has_comment,
            comment_is_block: &mut comment_is_block,
            has_docstring: &mut has_docstring,
        };
        visit(tree.root_node(), &mut ctx);

        let mut raw = RawLineCounts::default();
        classify_ts_lines(
            &lines,
            &has_code,
            &has_comment,
            &comment_is_block,
            &has_docstring,
            &mut raw,
        );

        Some(RawFileAnalysis {
            raw,
            parse_mode: ParseMode::TreeSitter,
            warnings: Vec::new(),
        })
    }

    /// Flags describing what kinds of content appear on a single line.
    // Four bools are the natural representation for these four independent properties.
    #[allow(clippy::struct_excessive_bools)]
    #[derive(Clone, Copy)]
    struct TsLineFlags {
        has_code: bool,
        has_comment: bool,
        comment_is_block: bool,
        has_docstring: bool,
    }

    /// Classify a single tree-sitter-annotated line and accumulate into `raw`.
    const fn classify_ts_line(trimmed: &str, flags: TsLineFlags, raw: &mut RawLineCounts) {
        if trimmed.is_empty() {
            raw.blank_only_lines += 1;
        } else if flags.has_docstring && !flags.has_code {
            raw.docstring_comment_lines += 1;
        } else if flags.has_code && flags.has_comment {
            // Classify the mixed line as single or multi based on what kind of comment is on it.
            if flags.comment_is_block {
                raw.mixed_code_multi_comment_lines += 1;
            } else {
                raw.mixed_code_single_comment_lines += 1;
            }
        } else if flags.has_comment {
            if flags.comment_is_block {
                raw.multi_comment_only_lines += 1;
            } else {
                raw.single_comment_only_lines += 1;
            }
        } else {
            raw.code_only_lines += 1;
        }
    }

    /// Classify each tree-sitter-annotated line and accumulate counts into `raw`.
    fn classify_ts_lines(
        lines: &[&str],
        has_code: &[bool],
        has_comment: &[bool],
        comment_is_block: &[bool],
        has_docstring: &[bool],
        raw: &mut RawLineCounts,
    ) {
        for i in 0..lines.len() {
            raw.total_physical_lines += 1;
            classify_ts_line(
                lines[i].trim(),
                TsLineFlags {
                    has_code: has_code[i],
                    has_comment: has_comment[i],
                    comment_is_block: comment_is_block[i],
                    has_docstring: has_docstring[i],
                },
                raw,
            );
        }
    }

    struct VisitCtx<'a> {
        source: &'a [u8],
        comment_kinds: &'a [&'a str],
        docstring_stmt_kind: Option<&'a str>,
        has_code: &'a mut Vec<bool>,
        has_comment: &'a mut Vec<bool>,
        comment_is_block: &'a mut Vec<bool>,
        has_docstring: &'a mut Vec<bool>,
    }

    /// Mark all rows of a comment node and detect whether it is a block comment.
    fn visit_comment_node(node: Node, ctx: &mut VisitCtx<'_>) {
        let start_row = node.start_position().row;
        let end_row = node.end_position().row;
        let first_two = node
            .utf8_text(ctx.source)
            .unwrap_or("")
            .get(..2)
            .unwrap_or("");
        let is_block = first_two == "/*" || first_two == "<#";
        for row in start_row..=end_row {
            if row < ctx.has_comment.len() {
                ctx.has_comment[row] = true;
                if is_block {
                    ctx.comment_is_block[row] = true;
                }
            }
        }
    }

    /// If `node` is an `expression_statement` whose sole named child is a string literal,
    /// mark those rows as docstring and return `true`.
    fn visit_maybe_docstring(node: Node, kind: &str, ctx: &mut VisitCtx<'_>) -> bool {
        let Some(stmt_kind) = ctx.docstring_stmt_kind else {
            return false;
        };
        if kind != stmt_kind || node.named_child_count() != 1 {
            return false;
        }
        let Some(child) = node.named_child(0) else {
            return false;
        };
        if child.kind() != "string" {
            return false;
        }
        let child_start = child.start_position().row;
        let child_end = child.end_position().row;
        for row in child_start..=child_end {
            if row < ctx.has_docstring.len() {
                ctx.has_docstring[row] = true;
            }
        }
        true
    }

    /// Mark all rows of a leaf (non-comment, non-extra) node as code.
    fn visit_leaf_code(node: Node, ctx: &mut VisitCtx<'_>) {
        let start_row = node.start_position().row;
        let end_row = node.end_position().row;
        for row in start_row..=end_row {
            if row < ctx.has_code.len() {
                ctx.has_code[row] = true;
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn visit(node: Node, ctx: &mut VisitCtx<'_>) {
        // NOSONAR
        let kind = node.kind();

        // Comment node — mark rows as comment, detect block vs. line comment.
        if ctx.comment_kinds.contains(&kind) {
            visit_comment_node(node, ctx);
            return;
        }

        // Python docstring: expression_statement whose only named child is a string literal.
        if visit_maybe_docstring(node, kind, ctx) {
            return;
        }

        // Leaf non-comment node: mark as code.
        if node.child_count() == 0 && !node.is_extra() {
            visit_leaf_code(node, ctx);
            return;
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                visit(child, ctx);
            }
        }
    }

    /// Parse C or C++ source with tree-sitter-c.
    #[must_use]
    pub fn analyze_c(text: &str) -> Option<RawFileAnalysis> {
        let lang: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
        analyze_lines(text, &lang, &["comment"], None)
    }

    /// Parse Python source with tree-sitter-python.
    #[must_use]
    pub fn analyze_python(text: &str) -> Option<RawFileAnalysis> {
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        analyze_lines(text, &lang, &["comment"], Some("expression_statement"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_docstrings_are_separated() {
        let input = r#""""module docs"""


def fn_a():
    """function docs"""
    value = 1  # trailing comment
    return value
"#;

        let result = analyze_text(Language::Python, input, AnalysisOptions::default());
        assert_eq!(result.raw.docstring_comment_lines, 2);
        assert_eq!(result.raw.mixed_code_single_comment_lines, 1);
        assert_eq!(result.raw.code_only_lines, 2);
    }

    #[test]
    fn c_style_mixed_lines_are_captured() {
        let input = "int x = 1; // note\n/* block */\n";
        let result = analyze_text(Language::C, input, AnalysisOptions::default());
        assert_eq!(result.raw.mixed_code_single_comment_lines, 1);
        assert_eq!(result.raw.multi_comment_only_lines, 1);
    }

    #[test]
    fn detect_language_by_shebang() {
        let language = detect_language(
            Path::new("script"),
            Some("#!/usr/bin/env bash"),
            &BTreeMap::new(),
            true,
        );
        assert_eq!(language, Some(Language::Shell));
    }
}
