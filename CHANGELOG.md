# Changelog

All notable changes to this project will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- Guided 4-step web UI wizard (select project → counting rules → outputs → review & run)
- Native directory picker via `rfd` (no path typing required)
- Light/dark theme toggle with system-preference detection
- Background watermark branding with randomised size, rotation, and placement
- Site footer with author credit and GitHub link on all web pages
- PDF export: non-blocking background generation via locally installed Chromium
- PDF export: dynamic `Content-Disposition` filename derived from report title
- PDF export: `--headless=old` fallback for Brave/newer Chromium builds
- Per-file breakdown in CLI output (`--per-file`)
- All four mixed-line policies selectable from CLI and web UI
- `oxidesloc report` command to re-render HTML/PDF from a saved JSON result
- `ci/` directory with three reusable `sloc.toml` presets:
  - `sloc-ci-default.toml` — balanced defaults mirroring web UI out of the box
  - `sloc-ci-strict.toml` — fail-fast, errors on binary files in source tree
  - `sloc-ci-full-scope.toml` — audit mode, counts everything including vendor/lockfiles
- Jenkins declarative pipeline (`Jenkinsfile`) with format → lint → test → build → smoke → archive stages
- GitHub Actions workflow (`ci.yml`) with quality gates, CLI smoke tests, and web UI health check
- GitHub Actions release workflow (`release.yml`) cross-compiling for Linux, Windows, and macOS (x86-64 + arm64)
- GitLab CI pipeline (`.gitlab-ci.yml`) with parallel smoke jobs and artifact retention
- Docker multi-stage build and `docker-compose.yml` for zero-dependency local deployment
- `Makefile` covering all common development tasks (`check`, `dev`, `build`, `serve`, `docker-*`, etc.)
- `.editorconfig` for consistent cross-editor formatting
- Community files: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `NOTICE`

### Languages supported
- C (`.c`, `.h`)
- C++ (`.cpp`, `.cc`, `.cxx`, `.hpp`)
- C# (`.cs`)
- Python (`.py`) with docstring classification
- Shell (`.sh`, `.bash`, `.zsh`, `.ksh`)
- PowerShell (`.ps1`, `.psm1`, `.psd1`)

---

## [0.1.0] — initial scaffold

### Added
- Rust workspace with six crates: `sloc-cli`, `sloc-config`, `sloc-core`, `sloc-languages`, `sloc-report`, `sloc-web`
- AGPL-3.0-or-later license
- Repository metadata and GitHub project setup
