# Changelog

All notable changes to oxide-sloc are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

---

## [1.0.0-rc.1] — 2026-04-25

> Release candidate for 1.0.0. Core feature set is complete. Please test and
> report issues — no breaking changes are expected between rc.1 and 1.0.0.

### Added

**Language support — 30 new languages (41 total)**
- Assembly (`.asm`, `.s`)
- Clojure (`.clj`, `.cljs`, `.cljc`, `.edn`)
- CSS (`.css`)
- Dart (`.dart`)
- Dockerfile (`Dockerfile`, `Dockerfile.*`)
- Elixir (`.ex`, `.exs`)
- Erlang (`.erl`, `.hrl`)
- F# (`.fs`, `.fsi`, `.fsx`)
- Groovy (`.groovy`, `.gradle`)
- Haskell (`.hs`, `.lhs`)
- HTML (`.html`, `.htm`, `.xhtml`)
- Julia (`.jl`)
- Kotlin (`.kt`, `.kts`)
- Lua (`.lua`)
- Makefile (`Makefile`, `GNUmakefile`, `.mk`)
- Nim (`.nim`, `.nims`)
- Objective-C (`.m`, `.mm`)
- OCaml (`.ml`, `.mli`)
- Perl (`.pl`, `.pm`, `.t`)
- PHP (`.php`)
- R (`.r`)
- Ruby (`.rb`, `.rake`, `Rakefile`, `Gemfile`)
- Scala (`.scala`, `.sc`)
- SCSS / Sass (`.scss`, `.sass`)
- SQL (`.sql`)
- Svelte (`.svelte`)
- Swift (`.swift`)
- Vue (`.vue`)
- XML / SVG (`.xml`, `.xsd`, `.xsl`, `.svg`)
- Zig (`.zig`)

**New output formats**
- `--csv-out <path>` on `analyze` and `report` — two-section CSV (summary + per-file)
- `--xlsx-out <path>` on `analyze` and `report` — multi-sheet Excel workbook (Summary, By Language, Per File, Skipped); self-contained ZIP+XML implementation, no external dependency
- `--csv-out` / `--xlsx-out` on `diff` — export delta as spreadsheet

**New CLI commands**
- `oxide-sloc diff <baseline.json> <current.json>` — compare two saved scans; prints colored delta summary; supports `--json-out`, `--csv-out`, `--xlsx-out`, `--plain`, `--quiet`
- `oxide-sloc init [PATH]` — generate a starter `.oxide-sloc.toml` with all options documented; `--force` to overwrite

**CLI improvements**
- Short flag aliases: `-j` (`--json-out`), `-H` (`--html-out`), `-c` (`--csv-out`), `-x` (`--xlsx-out`), `-q` (`--quiet`)
- `--open` on `analyze` and `report` — auto-opens the generated HTML in the system browser
- `--quiet` / `-q` — suppress all output except errors (useful in CI pipelines)
- `--fail-on-warnings` — exit with code 2 when warnings are present
- `--fail-below <N>` — exit with code 3 when code lines fall below threshold
- Colored terminal output when stdout is a TTY; suppressed by `NO_COLOR` env var or `--plain`
- Improved per-file and language-breakdown table formatting with aligned columns

**Release pipeline**
- `SHA256SUMS.txt` now included in every GitHub Release alongside the binaries

**Documentation**
- `CONTRIBUTING.md` — development workflow, vendor regeneration, PR checklist
- `SECURITY.md` — vulnerability disclosure policy and scope
- `CHANGELOG.md` (this file)

**Shebang detection extended** to Ruby, Perl, PHP, and Node.js scripts

---

## [0.2.0-beta.4] — 2026-04-24

### Changed
- Removed security commentary from source; pinned CI GitHub Actions to specific SHAs
- Applied `rustfmt` to `sloc-report` and `sloc-web` to pass CI format check
- Refreshed dist bundles (`[skip ci]`)

---

## [0.2.0-beta.3] — earlier

### Added
- `oxide-sloc serve --server` mode (binds `0.0.0.0`, suppresses browser auto-open)
- `oxide-sloc send` — SMTP and webhook delivery of saved JSON results
- Git metadata capture (`git_branch`, `git_commit_short/long`, `git_commit_author`, `git_tags`)
- Submodule breakdown (`--submodule-breakdown`)
- Delta computation in `sloc-core` (compare two `AnalysisRun` JSON files)
- Scan history/registry in `sloc-core` for the web UI
- PDF export via headless Chromium (`write_pdf_from_html`)
- Self-contained HTML report with light/dark theme toggle
- `run.sh` cross-platform launcher

### Fixed
- UTF-16 LE/BE and Windows-1252 encoding fallback during file discovery

---

## [0.1.0] — initial release

- CLI with `analyze`, `report`, `serve` subcommands
- JSON and HTML output formats
- 11 languages: C, C++, C#, Go, Java, JavaScript, Python, Rust, Shell, PowerShell, TypeScript
- Lexical state-machine analyzer with Python docstring classification
- Tree-sitter adapter scaffold (C and Python, behind `tree-sitter` feature flag)
- Axum web UI on `127.0.0.1:4317`
- GitHub Actions CI (fmt + clippy + build + test + smoke tests)
- Cross-platform release builds (Linux x86_64 musl, Windows x86_64 MSVC, macOS x86_64 + arm64)
