# Contributing to oxide-sloc

Thanks for your interest in contributing to oxide-sloc.

## Ground rules

- Keep changes focused and well-scoped.
- Prefer opening an issue before large feature work.
- Add or update tests when behavior changes.
- Document changes to public APIs and config schema.
- Use plain ASCII in generated source where practical.

## Development setup

```bash
rustup toolchain install stable
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## Commit guidance

Please make small, reviewable commits with clear messages.

## Licensing of contributions

Unless otherwise agreed in writing, contributions are submitted under the same license as this repository.
If the project later adopts a Contributor License Agreement, contributors may be asked to sign it before major contributions are merged.
