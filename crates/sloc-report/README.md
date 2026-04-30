# sloc-report

HTML report rendering and PDF export for [oxide-sloc](https://github.com/oxide-sloc/oxide-sloc).

## Overview

This crate handles all output rendering:

- **`render_html(run) -> Result<String>`** — Askama-templated HTML report with light/dark theme, language charts, per-file breakdown, and warning analysis
- **`write_pdf_from_html(html, path)`** — spawns a locally installed headless Chromium-based browser, polls for file stability, 45 s timeout
- Browser discovery: checks Chrome, Edge, Brave, Vivaldi, and Opera on Windows and Linux paths
- Set `SLOC_BROWSER` env var to override browser path; `SLOC_BROWSER_NOSANDBOX=1` passes `--no-sandbox` (required in Docker)

## Usage

This is an internal crate used by the oxide-sloc workspace. It is not intended for use outside this project. See the [main project](https://github.com/oxide-sloc/oxide-sloc) for documentation and releases.

```toml
# Install the tool instead:
cargo install oxide-sloc
```
