# Contributing to oxide-sloc

Thank you for your interest in improving oxide-sloc. This guide covers everything you need to get started.

## Quick start

```bash
git clone https://github.com/NimaShafie/oxide-sloc.git
cd oxide-sloc

# Decompress vendored dependencies (required for offline CI builds)
tar -xJf vendor.tar.xz

# Build the full workspace
cargo build --workspace

# Run the CI gates locally before pushing
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

## CI gates

All four must pass before a PR can merge:

| Check | Command |
|---|---|
| Format | `cargo fmt --all -- --check` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Build | `cargo build --workspace` |
| Tests | `cargo test --workspace` |

Run `cargo fmt --all` to auto-fix formatting issues.

## Adding a new language

Language support touches two crates and must be kept in sync:

1. **`crates/sloc-languages/src/lib.rs`**
   - Add a variant to `Language`
   - Add cases in `display_name()`, `as_slug()`, `from_name()`
   - Add extension / filename detection in `detect_language()`
   - Add a `ScanConfig` entry in `analyze_text()`
   - Add to `supported_languages()`

2. **`crates/sloc-config/src/lib.rs`** — verify that the new language name is accepted via `enabled_languages` filtering (it uses `Language::from_name` so no change is usually needed).

3. Add a small test file for the new language under `samples/basic/` so the smoke test covers it.

## Adding a new CLI flag

All flags live in `crates/sloc-cli/src/main.rs`. Follow the existing pattern: add the field to the relevant `*Args` struct, wire it in the `run_*` handler, and add it to the relevant `resolve_*_config` helper if it maps to a config field.

## Adding a new output format

Output writers live in `crates/sloc-report/src/lib.rs`. Export the public function from that crate, then call it from the CLI handler after the analysis run completes.

## Vendor directory

The `vendor/` directory is an offline mirror of all Cargo dependencies. When you add or upgrade a dependency:

```bash
# Regenerate the vendor snapshot and repack the archive
cargo vendor vendor
tar -cJf vendor.tar.xz vendor/
git add vendor.tar.xz
```

The CI decompresses `vendor.tar.xz` and builds entirely offline. Do not commit the expanded `vendor/` directory — only `vendor.tar.xz`.

## Commit messages

Follow the format used in `git log`:

```
type: short imperative summary (≤72 chars)
```

Common types: `feat`, `fix`, `refactor`, `docs`, `chore`, `ci`, `test`.

## Pull request checklist

- [ ] All four CI gates pass locally
- [ ] New language support updates both `sloc-languages` and has a sample file
- [ ] New dependencies are vendored (`vendor.tar.xz` updated)
- [ ] `CHANGELOG.md` updated under `[Unreleased]`
- [ ] No `#[allow(...)]` without a comment explaining why
- [ ] No `.unwrap()` or `.expect()` in library code

## License

By contributing you agree that your contributions will be licensed under the
project's [AGPL-3.0-or-later](LICENSE) license.
