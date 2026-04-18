# oxide-sloc

[![CI](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](./LICENSE)

**oxide-sloc** is a Rust-based source line analysis tool built for teams that want more than a simple line counter.

It is designed around one shared analysis core with multiple delivery surfaces:
- CLI
- localhost web UI
- rich HTML reports
- optional PDF export
- policy-aware counting for mixed code/comment lines and Python docstrings

## Repository

- Product name: **oxide-sloc**
- Binary name: **oxidesloc**
- License: **AGPL-3.0-or-later**
- GitHub: **https://github.com/NimaShafie/oxide-sloc**

## Positioning

oxide-sloc is intended to be a fast, extensible SLOC and line-analysis platform built in Rust for:
- local development
- CI/CD pipelines
- internal reporting
- future commercial packaging and support

This public repository is the main codebase for the project. A paid offering can still exist later through:
- commercial support
- hosted services
- proprietary add-ons in a separate codebase
- dual licensing for specific customers

## Current status

This repository is an early but runnable Rust workspace.

Working today:
- CLI analysis
- local web UI on localhost
- HTML report generation
- PDF export through a locally installed Chromium-based browser
- mixed-line policy handling
- Python docstring policy handling

Currently supported languages:
- C
- C++
- C#
- Python
- Shell
- PowerShell

Important:
If you run oxide-sloc against this Rust workspace itself, most files will currently be skipped because Rust, TOML, Markdown, YAML, and similar repository files are not yet supported.

Important note:
If you run oxide-sloc against this Rust workspace itself, you will currently see most files skipped as unsupported, because Rust/TOML/Markdown/YAML analysis is not implemented yet.

## Minimal local verification

Create a tiny sample directory with supported file types, then run:

cargo run -p oxidesloc -- analyze tmp-sloc --plain
cargo run -p oxidesloc -- analyze tmp-sloc --per-file
cargo run -p oxidesloc -- analyze tmp-sloc --json-out out/tmp.json --html-out out/tmp.html
cargo run -p oxidesloc -- serve

## Repository layout

```text
.
|-- crates/
|   |-- sloc-cli/         # CLI entrypoint and commands
|   |-- sloc-config/      # Shared config schema and validation
|   |-- sloc-core/        # Discovery, decoding, aggregation, JSON model
|   |-- sloc-languages/   # Language detection and analyzers
|   |-- sloc-report/      # HTML rendering and PDF export
|   `-- sloc-web/         # Localhost web UI
|-- docs/
|   `-- licensing.md
|-- .github/
|-- .gitignore
|-- CHANGELOG.md
|-- CODE_OF_CONDUCT.md
|-- CONTRIBUTING.md
|-- LICENSE
|-- LICENSE-COMMERCIAL.md
|-- NOTICE
|-- README.md
|-- SECURITY.md
`-- sloc.example.toml
```

## Licensing approach

This repository currently uses **AGPL-3.0-or-later**.

That means:
- the repository remains genuinely open source
- commercial use is allowed under the license terms
- modified networked deployments must provide corresponding source under AGPL terms
- you can still sell services, support, private add-ons, or separate commercial arrangements as the copyright holder

See `docs/licensing.md` for the practical tradeoffs and how this can evolve.

## Build

```bash
cargo build --workspace
```

## Run

Analyze a project and emit JSON and HTML:

```bash
cargo run -p oxidesloc -- analyze ./my-repo --json-out result.json --html-out result.html
```

Render a PDF from a saved JSON result:

```bash
cargo run -p oxidesloc -- report result.json --pdf-out result.pdf
```

Start the localhost web UI:

```bash
cargo run -p oxidesloc -- serve
```

## PDF export note

PDF generation currently depends on a locally installed Chromium-based browser. Set `SLOC_BROWSER` if browser discovery fails.

## Near-term roadmap

1. Run `cargo check` locally and fix dependency or API drift.
2. Add tree-sitter-backed adapters starting with Python and C/C++.
3. Add validation corpus and golden tests.
4. Add SMTP and webhook delivery support.
5. Publish release binaries and GitHub releases.

## Maintainer

Maintained by **Nima Shafie**.

## Legal note

This repository does not constitute legal advice. Before a public launch with outside contributors or a significant paid offering, review the licensing and commercial model with counsel.
