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
    pub fn display_name(&self) -> &'static str {
        match self {
            Language::C => "C",
            Language::Cpp => "C++",
            Language::CSharp => "C#",
            Language::Go => "Go",
            Language::Java => "Java",
            Language::JavaScript => "JavaScript",
            Language::Python => "Python",
            Language::Rust => "Rust",
            Language::Shell => "Shell",
            Language::PowerShell => "PowerShell",
            Language::TypeScript => "TypeScript",
            Language::Assembly => "Assembly",
            Language::Clojure => "Clojure",
            Language::Css => "CSS",
            Language::Dart => "Dart",
            Language::Dockerfile => "Dockerfile",
            Language::Elixir => "Elixir",
            Language::Erlang => "Erlang",
            Language::FSharp => "F#",
            Language::Groovy => "Groovy",
            Language::Haskell => "Haskell",
            Language::Html => "HTML",
            Language::Julia => "Julia",
            Language::Kotlin => "Kotlin",
            Language::Lua => "Lua",
            Language::Makefile => "Makefile",
            Language::Nim => "Nim",
            Language::ObjectiveC => "Objective-C",
            Language::Ocaml => "OCaml",
            Language::Perl => "Perl",
            Language::Php => "PHP",
            Language::R => "R",
            Language::Ruby => "Ruby",
            Language::Scala => "Scala",
            Language::Scss => "SCSS",
            Language::Sql => "SQL",
            Language::Svelte => "Svelte",
            Language::Swift => "Swift",
            Language::Vue => "Vue",
            Language::Xml => "XML",
            Language::Zig => "Zig",
        }
    }

    pub fn as_slug(&self) -> &'static str {
        match self {
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Go => "go",
            Language::Java => "java",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Rust => "rust",
            Language::Shell => "shell",
            Language::PowerShell => "powershell",
            Language::TypeScript => "typescript",
            Language::Assembly => "assembly",
            Language::Clojure => "clojure",
            Language::Css => "css",
            Language::Dart => "dart",
            Language::Dockerfile => "dockerfile",
            Language::Elixir => "elixir",
            Language::Erlang => "erlang",
            Language::FSharp => "fsharp",
            Language::Groovy => "groovy",
            Language::Haskell => "haskell",
            Language::Html => "html",
            Language::Julia => "julia",
            Language::Kotlin => "kotlin",
            Language::Lua => "lua",
            Language::Makefile => "makefile",
            Language::Nim => "nim",
            Language::ObjectiveC => "objectivec",
            Language::Ocaml => "ocaml",
            Language::Perl => "perl",
            Language::Php => "php",
            Language::R => "r",
            Language::Ruby => "ruby",
            Language::Scala => "scala",
            Language::Scss => "scss",
            Language::Sql => "sql",
            Language::Svelte => "svelte",
            Language::Swift => "swift",
            Language::Vue => "vue",
            Language::Xml => "xml",
            Language::Zig => "zig",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "c" => Some(Language::C),
            "cpp" | "c++" | "cplusplus" => Some(Language::Cpp),
            "csharp" | "c#" | "cs" => Some(Language::CSharp),
            "go" | "golang" => Some(Language::Go),
            "java" => Some(Language::Java),
            "javascript" | "js" => Some(Language::JavaScript),
            "python" | "py" => Some(Language::Python),
            "rust" | "rs" => Some(Language::Rust),
            "shell" | "sh" | "bash" => Some(Language::Shell),
            "powershell" | "pwsh" | "ps" => Some(Language::PowerShell),
            "typescript" | "ts" => Some(Language::TypeScript),
            "assembly" | "asm" => Some(Language::Assembly),
            "clojure" | "clj" => Some(Language::Clojure),
            "css" => Some(Language::Css),
            "dart" => Some(Language::Dart),
            "dockerfile" | "docker" => Some(Language::Dockerfile),
            "elixir" | "ex" => Some(Language::Elixir),
            "erlang" | "erl" => Some(Language::Erlang),
            "fsharp" | "f#" | "fs" => Some(Language::FSharp),
            "groovy" => Some(Language::Groovy),
            "haskell" | "hs" => Some(Language::Haskell),
            "html" | "htm" => Some(Language::Html),
            "julia" | "jl" => Some(Language::Julia),
            "kotlin" | "kt" => Some(Language::Kotlin),
            "lua" => Some(Language::Lua),
            "makefile" | "make" | "mk" => Some(Language::Makefile),
            "nim" => Some(Language::Nim),
            "objectivec" | "objc" | "objective-c" => Some(Language::ObjectiveC),
            "ocaml" | "ml" => Some(Language::Ocaml),
            "perl" | "pl" => Some(Language::Perl),
            "php" => Some(Language::Php),
            "r" => Some(Language::R),
            "ruby" | "rb" => Some(Language::Ruby),
            "scala" => Some(Language::Scala),
            "scss" | "sass" => Some(Language::Scss),
            "sql" => Some(Language::Sql),
            "svelte" => Some(Language::Svelte),
            "swift" => Some(Language::Swift),
            "vue" => Some(Language::Vue),
            "xml" => Some(Language::Xml),
            "zig" => Some(Language::Zig),
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

pub fn detect_language(
    path: &Path,
    first_line: Option<&str>,
    extension_overrides: &BTreeMap<String, String>,
    shebang_detection: bool,
) -> Option<Language> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    // Extension override check (user-configured mappings win over everything)
    if let Some(ext) = extension.as_ref() {
        if let Some(override_name) = extension_overrides.get(ext.as_str()) {
            if let Some(lang) = Language::from_name(override_name) {
                return Some(lang);
            }
        }
    }

    // Filename-based detection for files that have no extension or use exact names
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let filename_lower = filename.to_ascii_lowercase();

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

    let _ = stem; // suppress unused warning

    // Extension-based detection
    if let Some(ext) = extension.as_deref() {
        let by_ext = match ext {
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
        };

        if by_ext.is_some() {
            return by_ext;
        }
    }

    if shebang_detection {
        if let Some(line) = first_line {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("#!") {
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
            }
        }
    }

    None
}

pub fn analyze_text(language: Language, text: &str) -> RawFileAnalysis {
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
        ),
        Language::JavaScript => analyze_generic(
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
        ),
        Language::Dockerfile => analyze_generic(
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
        ),
        Language::Makefile => analyze_generic(
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
        ),
        Language::Svelte => analyze_generic(
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
        ),
        Language::Vue => analyze_generic(
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

#[derive(Debug, Clone, Copy)]
enum StringState {
    Single(char),
    Triple(&'static str),
    VerbatimDouble,
}

#[derive(Debug, Default)]
struct LineFacts {
    has_code: bool,
    has_single_comment: bool,
    has_multi_comment: bool,
    has_docstring: bool,
}

fn analyze_generic(text: &str, config: ScanConfig) -> RawFileAnalysis {
    let normalized = if text.is_empty() {
        String::new()
    } else {
        text.replace("\r\n", "\n").replace('\r', "\n")
    };

    let lines: Vec<&str> = if normalized.is_empty() {
        Vec::new()
    } else {
        normalized.split_terminator('\n').collect()
    };

    let mut raw = RawLineCounts::default();
    let mut warnings = Vec::new();

    let mut in_block_comment = false;
    let mut string_state: Option<StringState> = None;

    for (line_idx, line) in lines.iter().enumerate() {
        raw.total_physical_lines += 1;

        if config.skip_lines.contains(&line_idx) {
            raw.docstring_comment_lines += 1;
            continue;
        }

        let mut facts = LineFacts::default();
        let trimmed = line.trim();

        if in_block_comment {
            facts.has_multi_comment = true;
        }

        let chars: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if config.skip_lines.contains(&line_idx) {
                break;
            }

            if let Some(state) = string_state {
                facts.has_code = true;
                match state {
                    StringState::Single(delim) => {
                        if chars[i] == '\\' {
                            i += 2;
                            continue;
                        }
                        if chars[i] == delim {
                            string_state = None;
                        }
                        i += 1;
                        continue;
                    }
                    StringState::Triple(delim) => {
                        if starts_with(&chars, i, delim) {
                            string_state = None;
                            i += delim.len();
                        } else {
                            i += 1;
                        }
                        continue;
                    }
                    StringState::VerbatimDouble => {
                        if starts_with(&chars, i, "\"\"") {
                            i += 2;
                            continue;
                        }
                        if chars[i] == '"' {
                            string_state = None;
                        }
                        i += 1;
                        continue;
                    }
                }
            }

            if in_block_comment {
                facts.has_multi_comment = true;
                if let Some((_, close)) = config.block_comment {
                    if starts_with(&chars, i, close) {
                        in_block_comment = false;
                        i += close.len();
                    } else {
                        i += 1;
                    }
                    continue;
                }
            }

            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }

            if config.allow_csharp_verbatim_strings && starts_with(&chars, i, "@\"") {
                facts.has_code = true;
                string_state = Some(StringState::VerbatimDouble);
                i += 2;
                continue;
            }

            if config.allow_triple_quote_strings {
                if starts_with(&chars, i, "\"\"\"") {
                    facts.has_code = true;
                    string_state = Some(StringState::Triple("\"\"\""));
                    i += 3;
                    continue;
                }
                if starts_with(&chars, i, "'''") {
                    facts.has_code = true;
                    string_state = Some(StringState::Triple("'''"));
                    i += 3;
                    continue;
                }
            }

            if config.allow_single_quote_strings && chars[i] == '\'' {
                facts.has_code = true;
                string_state = Some(StringState::Single('\''));
                i += 1;
                continue;
            }

            if config.allow_double_quote_strings && chars[i] == '"' {
                facts.has_code = true;
                string_state = Some(StringState::Single('"'));
                i += 1;
                continue;
            }

            if let Some((open, _)) = config.block_comment {
                if starts_with(&chars, i, open) {
                    facts.has_multi_comment = true;
                    in_block_comment = true;
                    i += open.len();
                    continue;
                }
            }

            if let Some(prefix) = config
                .line_comments
                .iter()
                .find(|prefix| starts_with(&chars, i, prefix))
            {
                let _ = prefix;
                facts.has_single_comment = true;
                break;
            }

            facts.has_code = true;
            i += 1;
        }

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

        if facts.has_code {
            let (f, c, v, i) = count_symbols(&config.symbol_patterns, trimmed);
            raw.functions += f;
            raw.classes += c;
            raw.variables += v;
            raw.imports += i;
        }
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

fn count_symbols(patterns: &SymbolPatterns, trimmed: &str) -> (u64, u64, u64, u64) {
    let hit = |pats: &[&str]| pats.iter().any(|p| trimmed.starts_with(p)) as u64;
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

fn detect_python_docstring_lines(text: &str) -> HashSet<usize> {
    let normalized = if text.is_empty() {
        String::new()
    } else {
        text.replace("\r\n", "\n").replace('\r', "\n")
    };

    let lines: Vec<&str> = if normalized.is_empty() {
        Vec::new()
    } else {
        normalized.split_terminator('\n').collect()
    };

    #[derive(Debug, Clone)]
    struct PyContext {
        indent: usize,
        expect_docstring: bool,
    }

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

        if let Some((delim, start_line)) = active_docstring {
            docstring_lines.insert(idx);
            if closes_triple_docstring(trimmed, delim, idx == start_line) {
                active_docstring = None;
            }
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        while contexts.len() > 1 && indent < contexts.last().map(|c| c.indent).unwrap_or(0) {
            contexts.pop();
        }

        if let Some(base_indent) = pending_block_indent {
            if indent > base_indent {
                contexts.push(PyContext {
                    indent,
                    expect_docstring: true,
                });
                pending_block_indent = None;
            } else if !trimmed.starts_with('@') {
                pending_block_indent = None;
            }
        }

        if let Some(ctx) = contexts.last_mut() {
            if ctx.expect_docstring {
                if let Some(delim) = docstring_delimiter(trimmed) {
                    docstring_lines.insert(idx);
                    ctx.expect_docstring = false;
                    if !closes_triple_docstring(trimmed, delim, true) {
                        active_docstring = Some((delim, idx));
                    }
                    continue;
                }
                ctx.expect_docstring = false;
            }
        }

        if is_python_block_header(trimmed) {
            pending_block_indent = Some(indent);
        }
    }

    if let Some((_, start_line)) = active_docstring {
        for idx in start_line..lines.len() {
            docstring_lines.insert(idx);
        }
    }

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

/// Tree-sitter-backed adapters. Compiled only when the `tree-sitter` feature is enabled.
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
        ts_language: tree_sitter::Language,
        comment_node_kinds: &[&str],
        docstring_stmt_kind: Option<&str>,
    ) -> Option<RawFileAnalysis> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_language).ok()?;
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

        for i in 0..n {
            raw.total_physical_lines += 1;
            let trimmed = lines[i].trim();

            if trimmed.is_empty() {
                raw.blank_only_lines += 1;
            } else if has_docstring[i] && !has_code[i] {
                raw.docstring_comment_lines += 1;
            } else if has_code[i] && has_comment[i] {
                // Classify the mixed line as single or multi based on what kind of comment is on it.
                if comment_is_block[i] {
                    raw.mixed_code_multi_comment_lines += 1;
                } else {
                    raw.mixed_code_single_comment_lines += 1;
                }
            } else if has_comment[i] {
                if comment_is_block[i] {
                    raw.multi_comment_only_lines += 1;
                } else {
                    raw.single_comment_only_lines += 1;
                }
            } else {
                raw.code_only_lines += 1;
            }
        }

        Some(RawFileAnalysis {
            raw,
            parse_mode: ParseMode::TreeSitter,
            warnings: Vec::new(),
        })
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

    fn visit(node: Node, ctx: &mut VisitCtx<'_>) {
        let kind = node.kind();
        let start_row = node.start_position().row;
        let end_row = node.end_position().row;

        if ctx.comment_kinds.contains(&kind) {
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
            return;
        }

        // Python docstring: expression_statement whose only named child is a string literal
        if let Some(stmt_kind) = ctx.docstring_stmt_kind {
            if kind == stmt_kind && node.named_child_count() == 1 {
                if let Some(child) = node.named_child(0) {
                    if child.kind() == "string" {
                        let child_start = child.start_position().row;
                        let child_end = child.end_position().row;
                        for row in child_start..=child_end {
                            if row < ctx.has_docstring.len() {
                                ctx.has_docstring[row] = true;
                            }
                        }
                        return;
                    }
                }
            }
        }

        // Leaf non-comment node: mark as code.
        if node.child_count() == 0 && !node.is_extra() {
            for row in start_row..=end_row {
                if row < ctx.has_code.len() {
                    ctx.has_code[row] = true;
                }
            }
            return;
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                visit(child, ctx);
            }
        }
    }

    /// Parse C or C++ source with tree-sitter-c.
    pub fn analyze_c(text: &str) -> Option<RawFileAnalysis> {
        analyze_lines(text, tree_sitter_c::language(), &["comment"], None)
    }

    /// Parse Python source with tree-sitter-python.
    pub fn analyze_python(text: &str) -> Option<RawFileAnalysis> {
        analyze_lines(
            text,
            tree_sitter_python::language(),
            &["comment"],
            Some("expression_statement"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_docstrings_are_separated() {
        let input = r####""""module docs""""


def fn_a():
    """function docs"""
    value = 1  # trailing comment
    return value
"####;

        let result = analyze_text(Language::Python, input);
        assert_eq!(result.raw.docstring_comment_lines, 2);
        assert_eq!(result.raw.mixed_code_single_comment_lines, 1);
        assert_eq!(result.raw.code_only_lines, 2);
    }

    #[test]
    fn c_style_mixed_lines_are_captured() {
        let input = "int x = 1; // note\n/* block */\n";
        let result = analyze_text(Language::C, input);
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
