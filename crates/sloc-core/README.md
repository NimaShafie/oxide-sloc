# sloc-core

Core analysis engine for [oxide-sloc](https://github.com/oxide-sloc/oxide-sloc).

## Overview

This crate is the main entry point for analysis:

- **`analyze(config) -> Result<AnalysisRun>`** — file discovery, byte decoding (UTF-8 → UTF-16 → Windows-1252 fallback), language detection, per-file SLOC analysis, and aggregation
- **`AnalysisRun`** — canonical serializable result; read/write via `write_json` / `read_json`
- **`FileRecord`** — per-file details including effective counts after policy application
- Binary detection via long-line + low-whitespace heuristic
- Generated/vendor/lockfile skip logic
- Git submodule discovery and per-submodule breakdown

## Usage

This is an internal crate used by the oxide-sloc workspace. It is not intended for use outside this project. See the [main project](https://github.com/oxide-sloc/oxide-sloc) for documentation and releases.

```toml
# Install the tool instead:
cargo install oxide-sloc
```
