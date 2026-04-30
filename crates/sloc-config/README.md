# sloc-config

Configuration schema, TOML parsing, and CLI flag types for [oxide-sloc](https://github.com/oxide-sloc/oxide-sloc).

## Overview

This crate defines `AppConfig` and all nested configuration structs used across the oxide-sloc workspace:

- `DiscoveryConfig` — glob filters, ignore rules, submodule handling
- `AnalysisConfig` — IEEE 1045-1992 counting parameters (continuation lines, blank-in-block-comment policy, compiler directives)
- `ReportingConfig` — output paths, report title, mixed-line policy
- `WebConfig` — bind address, port, TLS, API key

Enums: `MixedLinePolicy`, `BinaryFileBehavior`, `FailureBehavior`, `ContinuationLinePolicy`, `BlankInBlockCommentPolicy`.

## Usage

This is an internal crate used by the oxide-sloc workspace. It is not intended for use outside this project. See the [main project](https://github.com/oxide-sloc/oxide-sloc) for documentation and releases.

```toml
# Install the tool instead:
cargo install oxide-sloc
```
