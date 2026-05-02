# oxide-sloc

[![CI](https://github.com/oxide-sloc/oxide-sloc/actions/workflows/ci.yml/badge.svg)](https://github.com/oxide-sloc/oxide-sloc/actions/workflows/ci.yml)
[![Release](https://github.com/oxide-sloc/oxide-sloc/actions/workflows/release.yml/badge.svg)](https://github.com/oxide-sloc/oxide-sloc/actions/workflows/release.yml)
[![Docker](https://github.com/oxide-sloc/oxide-sloc/actions/workflows/docker.yml/badge.svg)](https://github.com/oxide-sloc/oxide-sloc/actions/workflows/docker.yml)
[![Latest Release](https://img.shields.io/github/v/release/oxide-sloc/oxide-sloc?include_prereleases&label=release)](https://github.com/oxide-sloc/oxide-sloc/releases/latest)
[![crates.io](https://img.shields.io/crates/v/oxide-sloc.svg)](https://crates.io/crates/oxide-sloc)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](./LICENSE)

**oxide-sloc** is a Rust-based source line analysis tool — IEEE 1045-1992 compliant, more than a line counter.

## Quick Start

**Install via Cargo (requires Rust):**

```bash
cargo install oxide-sloc
oxide-sloc serve          # web UI at http://127.0.0.1:4317
oxide-sloc analyze ./my-repo --plain
```

**Or use a pre-built binary:**

| Platform | Install | Launch |
|---|---|---|
| **Windows 10/11** | `bash scripts/install.sh` (Git Bash) | `bash scripts/run.sh` (Git Bash) |
| **Linux — RHEL 8/9, Ubuntu, Debian** | `bash scripts/install.sh` | `bash scripts/run.sh` |

The install script uses a pre-built binary from `dist/` if present, or builds from vendored sources when Rust is available. On success, the web UI opens at **http://127.0.0.1:4317**.

For air-gapped setup, CI, and Docker, see [`docs/airgap.md`](./docs/airgap.md).

---

## Features

- **CLI** — `analyze / report / diff / serve / send / init` with a full flag set
- **Localhost web UI** — guided 4-step flow with light/dark theme, auto browser-open
- **Quick Scan** — one-click scan from step 1 using all defaults
- **Server mode** — `--server` binds to `0.0.0.0`, suppresses browser auto-open
- **IEEE 1045-1992 physical SLOC** — configurable counting parameters: mixed-line policy, continuation lines, compiler directives, blank-in-comment classification
- **Symbol counting** — lexical detection of functions, classes, variables, and imports per file
- **Rich HTML reports** — per-file breakdown, language charts, warning analysis
- **PDF export** — background generation via locally installed Chromium
- **CSV / Excel export** — from CLI flags or the report nav bar (4-sheet workbook)
- **Scan history & delta tracking** — every run is saved; re-scan to see lines added/removed
- **Side-by-side diff view** — compare any two scans with 4 chart types at `/compare`
- **Git submodule support** — per-submodule HTML sub-reports
- **Metrics API** — JSON endpoints for CI/CD dashboards
- **SVG badge endpoint** — embed live code-line counts in READMEs or Confluence
- **Embeddable summary widget** — `<iframe>` drop-in for internal wikis
- **Report delivery** — `send` command: SMTP email or JSON webhook POST
- **CI/CD ready** — Jenkinsfile, GitHub Actions, GitLab CI included
- **Docker image** — auto-published to GHCR on `main` and release tags
- **Air-gap / offline** — all crate dependencies vendored; Chart.js compiled in; no CDN calls
- **Confluence integration** — push HTML reports via REST API

---

## Installation

### Path A — Pre-built binary (no Rust required)

```bash
bash scripts/install.sh    # Windows 10/11 (Git Bash) or Linux
bash scripts/run.sh        # http://127.0.0.1:4317
```

The script tries in order: existing binary → `dist/` bundle → offline Rust build.

> **Transferable bundle:** Run `make bundle` to produce `oxide-sloc-bundle.tar.gz` — drop it on a USB drive and run `bash scripts/install.sh` on the target machine.

### Path B — Docker

```bash
docker pull ghcr.io/nimashafie/oxide-sloc:latest
# or build locally
export SLOC_API_KEY=$(openssl rand -hex 32)
docker compose up
```

```bash
# CLI via Docker
docker run --rm -v /path/to/your/repo:/repo:ro \
  ghcr.io/nimashafie/oxide-sloc:latest analyze /repo --plain
```

**Environment variables for `docker compose`:**

| Variable | Required | Description |
|---|---|---|
| `SLOC_API_KEY` | Yes | Bearer token for all web endpoints. Generate: `openssl rand -hex 32` |
| `SLOC_ALLOWED_ROOTS` | No | Colon-separated list of paths the web UI may scan. Default: unrestricted |
| `SLOC_TARGET` | No | Host directory to mount as `/repo`. Default: `./tmp-sloc` |
| `SLOC_TLS_CERT` / `SLOC_TLS_KEY` | No | Paths to PEM certificate and key for HTTPS |

See [`docs/airgap.md`](./docs/airgap.md) for air-gapped setup and [`docs/server-deployment.md`](./docs/server-deployment.md) for persistent deployments.

---

## Usage

### CLI

```bash
# Analyze and print a colored summary
oxide-sloc analyze ./my-repo

# Machine-readable key=value output
oxide-sloc analyze ./my-repo --plain

# Full output: JSON + HTML + CSV + Excel
oxide-sloc analyze ./my-repo -j result.json -H report.html -c report.csv -x report.xlsx

# Per-file breakdown in the terminal
oxide-sloc analyze ./my-repo --per-file

# Open the HTML report immediately after generation
oxide-sloc analyze ./my-repo -H report.html --open

# Quiet mode — only write files, print nothing (ideal for CI)
oxide-sloc analyze ./my-repo -j result.json --quiet

# Pipeline guards
oxide-sloc analyze ./my-repo --fail-on-warnings --fail-below 10000

# Filter by language or glob
oxide-sloc analyze ./my-repo --enabled-language rust --enabled-language python --plain
oxide-sloc analyze ./my-repo --include-glob "src/**" --exclude-glob "vendor/**"

# Git submodule breakdown
oxide-sloc analyze ./mono-repo --submodule-breakdown -j result.json -H report.html

# Re-render a report from saved JSON
oxide-sloc report result.json -H report.html --pdf-out report.pdf -c report.csv -x report.xlsx

# Compare two saved results
oxide-sloc diff baseline.json current.json
oxide-sloc diff baseline.json current.json -j delta.json -c delta.csv -x delta.xlsx

# Generate a starter config
oxide-sloc init                        # creates .oxide-sloc.toml
oxide-sloc init ci/sloc.toml --force

# Start the web UI
oxide-sloc serve              # http://127.0.0.1:4317, auto-opens browser
oxide-sloc serve --server     # binds to 0.0.0.0, no browser auto-open

# Deliver a saved report
oxide-sloc send result.json --smtp-to team@example.com --smtp-from bot@example.com --smtp-host smtp.example.com
oxide-sloc send result.json --webhook-url https://hooks.example.com/sloc --webhook-token "$TOKEN"
```

### CLI flags reference

#### `analyze`

| Flag | Short | Default | Description |
|---|---|---|---|
| `--json-out` | `-j` | *(none)* | Write JSON result |
| `--html-out` | `-H` | *(none)* | Write HTML report |
| `--csv-out` | `-c` | *(none)* | Write CSV summary |
| `--xlsx-out` | `-x` | *(none)* | Write Excel workbook (4 sheets) |
| `--pdf-out` | | *(none)* | Write PDF (requires Chrome/Edge/Brave) |
| `--open` | | off | Open HTML in system browser |
| `--quiet` | `-q` | off | Suppress all non-error output |
| `--plain` | | off | Machine-readable key=value output |
| `--per-file` | | off | Per-file breakdown in terminal |
| `--fail-on-warnings` | | off | Exit 2 if warnings are emitted |
| `--fail-below` | | *(none)* | Exit 3 if code lines fall below N |
| `--mixed-line-policy` | | `code-only` | `code-only` \| `code-and-comment` \| `comment-only` \| `separate-mixed-category` |
| `--python-docstrings-as-code` | | off | Treat docstrings as code |
| `--continuation-line-policy` | | `each-physical-line` | `each-physical-line` \| `collapse-to-logical` — IEEE 1045-1992 §3 |
| `--blank-in-block-comment-policy` | | `count-as-comment` | `count-as-comment` \| `count-as-blank` — IEEE 1045-1992 §4 |
| `--no-count-compiler-directives` | | off | Exclude `#include`/`#define` from code SLOC — IEEE 1045-1992 §4.2 (C/C++/ObjC only) |
| `--include-glob` | | *(all)* | Only scan matching files (repeatable) |
| `--exclude-glob` | | *(none)* | Skip matching files (repeatable) |
| `--enabled-language` | | *(all)* | Restrict to language (repeatable) |
| `--no-ignore-files` | | off | Ignore `.gitignore` / `.ignore` |
| `--follow-symlinks` | | off | Follow symbolic links |
| `--report-title` | | folder name | Title in HTML/PDF/XLSX reports |
| `--submodule-breakdown` | | off | Per-submodule stats from `.gitmodules` |
| `--config` | | *(none)* | Load settings from TOML file |

#### `report` / `diff` / `init` / `serve` / `send`

Run `oxide-sloc <command> --help` for the full flag list of each subcommand.

### Web UI

```bash
oxide-sloc serve   # → http://127.0.0.1:4317
```

A guided 4-step flow: select project → counting rules → outputs → review & run. The **Quick Scan** sidebar button submits from step 1 with all defaults.

Every web UI option maps 1:1 to a CLI flag — see the [Web UI → CLI translation](#web-ui--cli-translation) table below.

### Configuration file

```bash
cp examples/sloc.example.toml sloc.toml
oxide-sloc init    # or generate one with the CLI
```

CLI flags always override config file values.

---

## Scan history and delta tracking

Every web UI scan is recorded in `out/web/registry.json`. Re-running the same project path shows an inline delta:

- **Lines added / removed / unchanged**
- **Files modified / added / removed**

Navigate to `/history` to browse past scans, or `/compare?a=<run_id>&b=<run_id>` for a side-by-side file-level diff with four chart types.

---

## Symbol counting

oxide-sloc performs best-effort lexical detection of structural symbols across 10+ languages. Counts are surfaced in the JSON output (`functions`, `classes`, `variables`, `imports` fields) and in the HTML report.

Supported languages: C, C++, C#, Go, Java, JavaScript, Rust, Shell, PowerShell, TypeScript.

---

## Counting methodology — IEEE 1045-1992

oxide-sloc implements **physical SLOC** as defined in IEEE Std 1045-1992 *Software Productivity Metrics*. Every source line is classified into one of four categories before any policy is applied:

| Category | What it contains |
|---|---|
| **Code** | Executable statements, declarations, and compiler directives |
| **Comment** | Lines consisting solely of comment text |
| **Mixed** | Lines that contain both code and a trailing comment |
| **Blank** | Empty or whitespace-only lines |

The standard defines several counting parameters as configurable. oxide-sloc exposes all of them via CLI flags and the TOML config file.

### Mixed-line policy — `mixed_line_policy`

Controls how lines that contain both code and a comment are counted toward the totals. Default: `code-only`.

| Value | Behaviour |
|---|---|
| `code-only` *(default)* | Mixed lines count toward code only |
| `code-and-comment` | Mixed lines are counted in both totals |
| `comment-only` | Mixed lines count toward comments only |
| `separate-mixed-category` | Mixed lines are kept in a separate total |

### Continuation-line policy — `continuation_line_policy` (IEEE 1045-1992 §3)

Controls how backslash-continued lines (C/C++ macros, shell, Makefile) are counted. Default: `each-physical-line`.

| Value | Behaviour |
|---|---|
| `each-physical-line` *(default)* | Each physical line is counted separately (physical SLOC mode) |
| `collapse-to-logical` | A backslash-continued sequence counts as a single logical line |

### Blank lines inside block comments — `blank_in_block_comment_policy` (IEEE 1045-1992 §4)

Controls how blank lines that fall inside `/* ... */` (or equivalent) comment blocks are classified. Default: `count-as-comment`, which is the IEEE-aligned behaviour.

| Value | Behaviour |
|---|---|
| `count-as-comment` *(default, IEEE aligned)* | Blank lines inside block comments count as comment lines |
| `count-as-blank` | Blank lines inside block comments remain blank lines |

### Compiler directives — `count_compiler_directives` (IEEE 1045-1992 §4.2)

Applies to **C, C++, and Objective-C** only. By default, preprocessor directive lines (`#include`, `#define`, `#ifdef`, `#pragma`, etc.) are counted as code lines. Set `count_compiler_directives = false` to exclude them from effective code SLOC — they are still recorded in the raw JSON output as `compiler_directive_lines` so nothing is lost.

### TOML configuration

All parameters are settable in `.oxide-sloc.toml` under `[analysis]`:

```toml
[analysis]
mixed_line_policy            = "code-only"          # code-only | code-and-comment | comment-only | separate-mixed-category
continuation_line_policy     = "each-physical-line"  # each-physical-line | collapse-to-logical
blank_in_block_comment_policy = "count-as-comment"  # count-as-comment | count-as-blank
count_compiler_directives    = true                 # false = exclude #include/#define from code SLOC (C/C++/ObjC)
python_docstrings_as_comments = true                # false = treat docstrings as code
```

Run `oxide-sloc init` to generate a starter config with all options documented inline.

---

## Supported languages (41)

| Language | Extensions / Filenames | Comment styles |
|---|---|---|
| Assembly | `.asm`, `.s` | `;` |
| C | `.c`, `.h` | `//` `/* */` |
| C++ | `.cc`, `.cpp`, `.cxx`, `.hpp`, `.hxx` | `//` `/* */` |
| C# | `.cs` | `//` `/* */` verbatim strings |
| Clojure | `.clj`, `.cljs`, `.cljc`, `.edn` | `;` |
| CSS | `.css` | `/* */` |
| Dart | `.dart` | `//` `/* */` |
| Dockerfile | `Dockerfile`, `Dockerfile.*` | `#` |
| Elixir | `.ex`, `.exs` | `#` |
| Erlang | `.erl`, `.hrl` | `%` |
| F# | `.fs`, `.fsi`, `.fsx` | `//` `(* *)` |
| Go | `.go` | `//` `/* */` |
| Groovy | `.groovy`, `.gradle` | `//` `/* */` |
| Haskell | `.hs`, `.lhs` | `--` `{- -}` |
| HTML | `.html`, `.htm`, `.xhtml` | `<!-- -->` |
| Java | `.java` | `//` `/* */` |
| JavaScript | `.js`, `.mjs`, `.cjs` | `//` `/* */` |
| Julia | `.jl` | `#` `#= =#` |
| Kotlin | `.kt`, `.kts` | `//` `/* */` |
| Lua | `.lua` | `--` `--[[ ]]` |
| Makefile | `Makefile`, `GNUmakefile`, `.mk` | `#` |
| Nim | `.nim`, `.nims` | `#` `#[ ]#` |
| Objective-C | `.m`, `.mm` | `//` `/* */` |
| OCaml | `.ml`, `.mli` | `(* *)` |
| Perl | `.pl`, `.pm`, `.t` | `#` |
| PHP | `.php` | `//` `#` `/* */` |
| PowerShell | `.ps1`, `.psm1`, `.psd1` | `#` `<# #>` |
| Python | `.py` | `#` docstrings |
| R | `.r` | `#` |
| Ruby | `.rb`, `.rake`, `Rakefile`, `Gemfile` | `#` |
| Rust | `.rs` | `//` `/* */` |
| Scala | `.scala`, `.sc` | `//` `/* */` |
| SCSS | `.scss`, `.sass` | `//` `/* */` |
| Shell | `.sh`, `.bash`, `.zsh`, `.ksh` | `#` |
| SQL | `.sql` | `--` `/* */` |
| Svelte | `.svelte` | `//` `/* */` |
| Swift | `.swift` | `//` `/* */` |
| TypeScript | `.ts`, `.mts`, `.cts` | `//` `/* */` |
| Vue | `.vue` | `//` `/* */` |
| XML / SVG | `.xml`, `.xsd`, `.xsl`, `.svg` | `<!-- -->` |
| Zig | `.zig` | `//` |

> **Not supported (intentionally):** TOML, Markdown, YAML — no meaningful SLOC metric applies.
> Shebang (`#!`) detection works for Python, Shell, Ruby, Perl, PHP, and Node.js scripts.

### Adding a new language

1. **`crates/sloc-languages/src/lib.rs`** — add a `Language` variant, implement `display_name`/`as_slug`/`from_name`, register extensions in `detect_language`, add a `ScanConfig` entry in `analyze_text`.
2. No change needed in `sloc-config` — `enabled_languages` filtering picks up new variants automatically.

---

## PDF export

PDF generation uses a locally installed Chromium-based browser (Chrome, Edge, Brave, Vivaldi, or Opera). Generation runs in the background; the web UI returns results immediately.

```bash
export SLOC_BROWSER=/usr/bin/chromium   # override browser path
oxide-sloc report result.json --pdf-out result.pdf
```

In Docker, Chromium is bundled — no extra setup needed.

---

## CSV and Excel export

Every HTML report has **Export CSV** and **Export Excel** buttons in the nav bar. The Excel workbook contains four sheets: **Summary**, **By Language**, **Per File**, and **Skipped Files** — no plugins required, works in Excel, LibreOffice, and Google Sheets.

The same exports are available from the CLI:

```bash
oxide-sloc analyze ./my-repo -c result.csv -x result.xlsx
oxide-sloc report result.json -c result.csv -x result.xlsx
oxide-sloc diff baseline.json current.json -c delta.csv -x delta.xlsx
```

---

## Metrics API

| Endpoint | Description |
|---|---|
| `GET /api/metrics/latest` | Metrics for the most recent scan |
| `GET /api/metrics/:run_id` | Metrics for a specific run |
| `GET /api/project-history?path=<dir>` | Scan history for a project root |
| `GET /badge/:metric` | SVG badge (`code-lines`, `files`, `comment-lines`, `blank-lines`) |
| `GET /embed/summary` | Embeddable HTML widget |
| `GET /healthz` | Health check |

```markdown
![Code Lines](http://your-host:4317/badge/code-lines)
```

```html
<iframe src="http://your-host:4317/embed/summary" width="100%" height="180" frameborder="0"></iframe>
```

---

## CI/CD

### Web UI → CLI translation

| Web UI | CLI equivalent |
|---|---|
| Step 1: select project | `oxide-sloc analyze ./my-repo` |
| Step 1: include / exclude pattern | `--include-glob` / `--exclude-glob` |
| Step 1: submodule breakdown | `--submodule-breakdown` |
| Quick Scan | `oxide-sloc analyze ./my-repo --plain` |
| Step 2: mixed-line policy | `--mixed-line-policy code-only` |
| Step 2: Python docstrings as code | `--python-docstrings-as-code` |
| *(config / CLI only)* | `--continuation-line-policy collapse-to-logical` |
| *(config / CLI only)* | `--blank-in-block-comment-policy count-as-blank` |
| *(config / CLI only)* | `--no-count-compiler-directives` |
| Step 3: outputs | `-j` `-H` `--pdf-out` `-c` `-x` `--open` |
| Step 3: custom title | `--report-title "My Report"` |
| Re-render from saved JSON | `oxide-sloc report result.json -H report.html` |
| Compare two scans | `oxide-sloc diff baseline.json current.json` |
| Generate starter config | `oxide-sloc init` |
| Quiet / fail guards | `--quiet` `--fail-on-warnings` `--fail-below N` |

### CI config presets

| File | Use case |
|---|---|
| `ci/sloc-ci-default.toml` | Balanced defaults |
| `ci/sloc-ci-strict.toml` | Fail-fast on binary files |
| `ci/sloc-ci-full-scope.toml` | Audit mode — counts vendor/lockfiles too |

### GitHub Actions

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to `main`, all PRs | fmt → clippy → build → tests → CLI smoke → web health check |
| `release.yml` | `v*` tag | Cross-compile for 5 platforms → sign Windows binary → GitHub Release |
| `docker.yml` | push to `main`, `v*` tag | Build and push Docker image to GHCR |
| `update-dist.yml` | `v*` tag, manual | Build platform bundles and commit to `dist/` |

All workflows run on Node 24.

To cut a release:

```bash
git tag v1.1.0
git push origin v1.1.0
```

### Jenkins / GitLab CI

A `Jenkinsfile` and `.gitlab-ci.yml` are included at the repo root. On self-hosted or air-gapped runners, download `vendor.tar.xz` from the release page, place it in the workspace, and the pipeline will decompress and cache `vendor/` between runs.

For detailed setup including Confluence publishing, see [`docs/ci-integrations.md`](./docs/ci-integrations.md).

---

## Local development

```bash
# Run all CI gates before pushing
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace

# Run the web UI during development
cargo run -p oxide-sloc -- serve

# Fast rebuild (keeps vendored dep cache, ~1 min)
cargo clean -p oxide-sloc -p sloc-config -p sloc-core -p sloc-languages -p sloc-report -p sloc-web \
  && cargo run -p oxide-sloc -- serve
```

> **`scripts/run.sh` vs `cargo run`:** When Rust is available, `scripts/run.sh` prefers `cargo run` so changes are always picked up. During active development, either works.

**Make targets (Linux/macOS):**

```bash
make check        # fmt + lint + test
make dev          # fmt + lint + test + serve
make build        # release binary → target/release/oxide-sloc
make docker-build # build Docker image locally
```

---

## Repository layout

```
crates/
  sloc-cli/         # CLI entry point and commands
  sloc-config/      # Config schema and TOML parsing
  sloc-core/        # File discovery, decoding, aggregation, delta engine
  sloc-languages/   # Language detection, lexical analyzers, symbol counting
  sloc-report/      # HTML rendering, PDF export, CSV/Excel export
  sloc-web/         # Axum web server, scan registry, metrics API, badge endpoint
ci/                 # CI shell scripts (lint.sh, build.sh, test.sh, release.sh) + config presets
deploy/             # systemd unit + server config template
dist/               # Release bundles — generated by CI, not tracked in git
docs/
  assets/           # Icons, logos (served at /images/* by the web UI)
  airgap.md         # Offline and air-gapped deployment guide
  ci-integrations.md
  server-deployment.md
examples/           # Runnable examples + sloc.example.toml config template
scripts/            # install.sh, run.sh, airgap-build.sh, update-vendor.sh
tests/
  fixtures/basic/   # Sample source files used by smoke tests
```

---

## License

**oxide-sloc** is licensed under [AGPL-3.0-or-later](./LICENSE).
Copyright (C) 2026 Nima Shafie. All intellectual property rights vest solely in the author.

Commercial support, hosted services, and proprietary add-ons are available through separate arrangements. See [`docs/licensing-commercial.md`](./docs/licensing-commercial.md).

---

**Nima Shafie** — [github.com/NimaShafie](https://github.com/NimaShafie)
