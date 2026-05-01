# Changelog

All notable changes to oxide-sloc are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

### Documentation

- **Jenkins bootstrap gaps closed**: Added `ci/jenkins/.env.example` for local credential storage and `ci/jenkins/preflight.sh` pre-flight check script.
- Added "Obtaining credentials" section to `ci/jenkins/README.md` and `docs/ci-integrations.md` covering initial admin password retrieval (native and Docker installs) and API token minting click-path.
- Added native/systemd plugin install path (Jenkins CLI jar) as Path 3 in `ci/jenkins/plugins.txt` and `ci/jenkins/README.md`.
- Rewrote the CLI bootstrap snippet in both docs to use `JENKINS_TOKEN` (sourced from `ci/jenkins/.env`) instead of the bare `JENKINS_PASS` placeholder; dropped the unnecessary cookie jar from token-based authentication.
- Added explicit seed-build curl (`POST /job/${JOB_NAME}/build`) with note that the first build seeds the parameters form.
- Added note that LAN/remote URLs (e.g., `http://10.0.0.8:8080`) are valid and that trailing slashes must be stripped.
- Added job-name decision rule: use `oxide-sloc` for the SCM-driven job; use `oxide-sloc-manual` only when maintaining a parallel hand-edited job in the same instance.
- Added `ci/jenkins/.env` to `.gitignore`.

---

## [1.2.7] — 2026-05-01

### Changed

- Resolved 485 SonarQube findings (16 HIGH, 469 MEDIUM) across all 6 crates with zero remaining actionable issues:
  - Replaced 139 unnecessary struct-name repetitions with `Self` in `impl` blocks
  - Converted 41 `push_str(&format!(...))` calls to `write!()` to avoid intermediate allocations
  - Fixed 22 case-sensitive file-extension comparisons to use `eq_ignore_ascii_case`
  - Added `# Errors` doc sections to all 15 public `Result`-returning functions missing them
  - Merged identical `match` arms, removed redundant closures, inlined format args, and applied `let…else` rewrites throughout
  - Tightened mutex-guard scopes in four `sloc-web` handlers (`significant_drop_tightening`)
  - Converted `resolve_output_root` from `Result<PathBuf>` to `PathBuf` (unnecessary wrap removed)
  - Added `#[allow]` with explanatory context for deliberate narrowing casts in ZIP generation, calendar math, and badge-pixel arithmetic
  - Added intent comment to Python test corpus fixture (`mixed.py`) for empty method

---

## [1.2.0] — 2026-04-29

### Added

**IEEE 1045-1992 physical SLOC compliance**
- The counting engine now implements all configurable parameters defined in IEEE Std 1045-1992 *Software Productivity Metrics*:
  - `continuation_line_policy` / `--continuation-line-policy` (`each-physical-line` | `collapse-to-logical`) — IEEE §3: optionally collapse backslash-continued C macro / shell / Makefile lines into a single logical line count instead of counting each physical line
  - `blank_in_block_comment_policy` / `--blank-in-block-comment-policy` (`count-as-comment` | `count-as-blank`) — IEEE §4: blank lines inside `/* ... */` blocks are classified as comment lines by default (IEEE aligned); `count-as-blank` restores legacy behaviour if needed
  - `count_compiler_directives` / `--no-count-compiler-directives` — IEEE §4.2: `#include`, `#define`, `#ifdef`, and other C/C++/Objective-C preprocessor directive lines are now tracked separately as `compiler_directive_lines` in the raw JSON output; passing `--no-count-compiler-directives` (or setting `count_compiler_directives = false`) excludes them from effective code SLOC while keeping the raw count intact
  - All three parameters are settable in `.oxide-sloc.toml` under `[analysis]` and via CLI flags on `analyze`

**Web server security hardening**
- IP-based sliding-window rate limiter (60 requests / 60 seconds per client IP) across all routes — no external crate required; uses only `std` + `Instant`
- Bearer-token authentication via `SLOC_API_KEY` env var — when set, all requests must supply a matching `Authorization: Bearer <key>` or `X-API-Key: <key>` header; startup warning logged when running in server mode without a key
- Native TLS termination via `SLOC_TLS_CERT` / `SLOC_TLS_KEY` PEM env vars (powered by `tokio-rustls` + `rustls`); startup warning logged when `--server` is used without TLS configured
- CORS headers via `tower-http::CorsLayer`
- Security response headers middleware (X-Content-Type-Options, X-Frame-Options, Referrer-Policy, etc.)
- Graceful shutdown on `Ctrl+C` (both local and server modes)

**New web routes**
- `GET /view-reports` — scan history browser
- `GET /compare-scans` — side-by-side scan comparison UI
- `GET /embed/summary` — embeddable summary widget (iframe-friendly)

**Webhook security**
- `validate_webhook_url()` now enforces HTTPS and blocks SSRF targets (loopback, RFC-1918 private ranges, link-local, cloud metadata endpoints: `169.254.169.254`, `metadata.google.internal`, `*.local`)

**SMTP credential safety**
- `--smtp-pass` on the `send` command now emits a visible warning when used directly; use `SLOC_SMTP_PASS` env var instead to keep credentials out of process listings

**CI/CD hardening**
- Docker builder and runtime images pinned to SHA-256 digests (`rust:slim@sha256:…`, `debian:bookworm-slim@sha256:…`) — prevents silent base-image substitution
- GitLab CI pipeline switched from curl-piped rustup to the official `rust:slim` pinned image
- `vendor.tar.xz` integrity verified via `sha256sum -c vendor.tar.xz.sha256` before extraction in Dockerfile, GitLab CI, and Jenkinsfile
- Docker image signed with `cosign` (keyless OIDC) and SBOM attached via `docker/build-push-action`; `id-token: write` permission added to `docker.yml`
- Jenkins parameter injection hardening: `SCAN_PATH`, `REPORT_TITLE`, `MIXED_LINE_POLICY` passed through `withEnv` (shell variables, not Groovy interpolation); allowlist validation added for choice and free-text parameters (`MIXED_LINE_POLICY`, `CI_PRESET`, `OUTPUT_SUBDIR`, glob patterns, language names)
- Jenkins CSP relaxation rationale documented inline; alternative of serving HTML from a separate origin noted for high-assurance environments

**Docker**
- `HEALTHCHECK` instruction added — polls `GET /healthz` every 30 s; 5 s timeout; 3 retries
- `SLOC_BROWSER_NOSANDBOX=1` env var added to Docker image — bypasses Chromium kernel-namespace sandbox (required in most container runtimes without `SYS_ADMIN`); documented with guidance on when to disable it
- `wget` added to runtime image (required by `HEALTHCHECK`)

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
