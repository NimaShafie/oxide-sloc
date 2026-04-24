# oxide-sloc

[![CI](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml)
[![Release](https://github.com/NimaShafie/oxide-sloc/actions/workflows/release.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/release.yml)
[![Docker](https://github.com/NimaShafie/oxide-sloc/actions/workflows/docker.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/docker.yml)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](./LICENSE)

**oxide-sloc** is a Rust-based source line analysis tool built for teams that want more than a simple line counter.

## Quick Start — no Rust, no dependencies

Pre-built binaries are bundled in `dist/`. Transfer the repository folder to any machine and run:

| Platform | Command | Result |
|---|---|---|
| **Windows 10/11** | Double-click `run.bat` | Extracts binary, starts web UI |
| **Linux — RHEL 8/9, Ubuntu, Debian** | `bash run.sh` | Extracts binary, starts web UI |

The web UI opens at **http://127.0.0.1:4317**. That's it — no setup, no install step, no internet.

For step-by-step instructions and the source-build path, see [`docs/airgap.md`](./docs/airgap.md).

---

## Features

One shared analysis core with multiple delivery surfaces:

- **CLI** — `oxidesloc analyze / report / serve` with a full flag set
- **Quick Scan** — one-click scan from the web UI with zero configuration
- **Localhost web UI** — guided 4-step flow with light/dark theme, auto browser-open
- **Rich HTML reports** — per-file breakdown, language summaries, warning analysis, high-value support opportunities
- **PDF export** — non-blocking background generation via locally installed Chromium
- **Export to CSV / Excel** — download per-file data from any HTML report via nav bar buttons
- **Scan history & delta tracking** — every run is saved; re-scan to see lines added/removed/unchanged
- **Side-by-side diff view** — compare any two historical scans at the file level (`/compare`)
- **Policy-aware counting** — mixed code/comment lines, Python docstrings
- **Git submodule support** — auto-detect `.gitmodules` and produce per-submodule HTML sub-reports
- **Metrics API** — JSON endpoints for CI/CD dashboards and custom tooling
- **SVG badge endpoint** — embed live code-line counts in READMEs, Confluence pages, and Jira
- **Embeddable summary widget** — drop an `<iframe>` into any internal wiki page
- **CI/CD ready** — Jenkinsfile, GitHub Actions, and GitLab CI pipelines included
- **Docker image** — auto-published to GHCR on every push to `main` and on every release tag
- **Air-gap / offline** — all 328 crate dependencies vendored; Chart.js compiled in; no CDN calls ever
- **Confluence integration** — push HTML reports or summary tables via REST API

---

## Installation

### Option 1 — Bundled binary, zero setup (recommended)

The fastest path — no download step needed. The repository already contains pre-built
binaries in `dist/` for Windows and Linux.

```
# Windows 10/11
run.bat

# Linux (RHEL 8/9, Ubuntu 18+, Debian 10+)
bash run.sh
```

Both scripts extract the correct binary and start the web UI at http://127.0.0.1:4317.
See [`docs/airgap.md`](./docs/airgap.md) for the full offline/air-gap guide.

### Option 2 — Download from GitHub Releases (no Rust required)

For platforms beyond Windows and Linux x86-64 (e.g. macOS), download from the
[Releases page](https://github.com/NimaShafie/oxide-sloc/releases):

| Platform | File |
|---|---|
| Linux x86-64 (static) | `oxide-sloc-linux-x86_64` |
| Windows x86-64 | `oxide-sloc-windows-x86_64.exe` |
| macOS x86-64 | `oxide-sloc-macos-x86_64` |
| macOS Apple Silicon | `oxide-sloc-macos-arm64` |

```bash
# Linux / macOS — make executable and move to PATH
chmod +x oxide-sloc-linux-x86_64
mv oxide-sloc-linux-x86_64 /usr/local/bin/oxidesloc

# Windows — rename and place in PATH
ren oxide-sloc-windows-x86_64.exe oxidesloc.exe
```

### Option 3 — Docker (no Rust required)

Pull the pre-built image from GitHub Container Registry:

```bash
docker pull ghcr.io/nimashafie/oxide-sloc:latest
```

Or build locally:

```bash
docker compose up
```

Open [http://localhost:4317](http://localhost:4317) in your browser.

> **Note:** The first local build takes a few minutes. The GHCR image starts instantly.

To analyze a directory from the CLI via Docker:

```bash
docker run --rm \
  -v /path/to/your/repo:/repo:ro \
  ghcr.io/nimashafie/oxide-sloc:latest \
  analyze /repo --plain
```

Chromium is bundled in the Docker image — PDF export works out of the box.

### Option 4 — Build from source (requires Rust 1.78+)

```bash
cargo install --path crates/sloc-cli
```

Or build without installing:

```bash
cargo build --release -p oxidesloc
./target/release/oxidesloc --help
```

All 328 crate dependencies are vendored in `vendor/` and `.cargo/config.toml` is pre-configured to use them — `cargo build` requires no network access after cloning.

---

## Usage

### CLI

```bash
# Analyze a directory — print summary to terminal
oxidesloc analyze ./my-repo --plain

# Full output: JSON + HTML report
oxidesloc analyze ./my-repo \
  --json-out result.json \
  --html-out result.html

# Per-file breakdown
oxidesloc analyze ./my-repo --per-file --plain

# Apply a specific counting policy
oxidesloc analyze ./my-repo --mixed-line-policy separate-mixed-category --plain

# Include/exclude file patterns
oxidesloc analyze ./my-repo \
  --include-glob "src/**" \
  --exclude-glob "vendor/**" \
  --plain

# Custom report title
oxidesloc analyze ./my-repo \
  --report-title "Q2 Code Review" \
  --html-out report.html

# Scan a super-repository — detect git submodules and report each separately
oxidesloc analyze ./mono-repo \
  --submodule-breakdown \
  --json-out result.json \
  --html-out report.html

# Re-render a report from a saved JSON (change format without re-scanning)
oxidesloc report result.json --html-out report.html --pdf-out report.pdf

# Start the web UI (auto-opens browser)
oxidesloc serve
```

### CLI flags reference

| Flag | Values | Default | Description |
|---|---|---|---|
| `--mixed-line-policy` | `code-only` `code-and-comment` `comment-only` `separate-mixed-category` | `code-only` | How lines containing both code and inline comments are classified |
| `--python-docstrings-as-code` | *(flag)* | off | Treat docstrings as code instead of comments |
| `--include-glob` | glob pattern | *(all)* | Only scan files matching this pattern (repeatable) |
| `--exclude-glob` | glob pattern | *(none)* | Skip files matching this pattern (repeatable) |
| `--report-title` | string | folder name | Title shown in HTML/PDF reports |
| `--json-out` | path | *(none)* | Write JSON analysis result to file |
| `--html-out` | path | *(none)* | Write HTML report to file |
| `--pdf-out` | path | *(none)* | Write PDF report to file |
| `--per-file` | *(flag)* | off | Include per-file breakdown in terminal output |
| `--plain` | *(flag)* | off | Plain terminal output (no color) |
| `--submodule-breakdown` | *(flag)* | off | Detect `.gitmodules` and emit per-submodule stats |
| `--config` | path | `sloc.toml` | Load settings from TOML config file |

### Web UI

```bash
oxidesloc serve
# → http://127.0.0.1:4317  (opens automatically)
```

The web UI is a guided 4-step flow with an optional one-click fast path:

| Step | What it configures |
|---|---|
| **1 — Select project** | Target folder, include/exclude glob patterns, git submodule breakdown, live scope preview |
| **2 — Counting rules** | Mixed-line policy, Python docstring handling, generated/minified/vendor/lockfile/binary file behavior |
| **3 — Outputs and reports** | Scan preset, artifact preset, output directory, report title |
| **4 — Review and run** | Summary of all settings, one-click scan |

### Quick Scan

The sidebar includes a **Quick Scan** button that submits the form immediately from Step 1 using all default settings. Use it when you do not need to customize counting rules or output options — just browse to your folder and click Quick Scan.

Everything available in the web UI maps directly to a CLI flag — see [Web UI → CLI translation](#web-ui--cli-translation).

### Configuration file

Copy the example config and edit it:

```bash
cp sloc.example.toml sloc.toml
```

CLI flags always override config file values. Run `oxidesloc --help` for the full flag list.

---

## Scan history and delta tracking

Every scan run through the web UI is recorded in an on-disk registry (`out/web/registry.json` by default). Re-running a scan on the same project path automatically computes a line-level delta:

- **Lines added** — new code lines since the previous scan
- **Lines removed** — code lines that no longer exist
- **Unmodified lines** — lines present in both scans
- **Files modified / added / removed** — file-level change summary

The result page displays the delta inline and offers a **Full diff →** link to the side-by-side compare view.

### Compare view

Navigate to `/history` to browse all past scans. Select any two runs and click **Compare** to open a file-by-file diff showing code delta per file. You can also reach the compare view from the result page via the **Full diff →** button shown whenever a previous scan exists for the same project.

```
/history                        → scan history browser
/compare-select                 → select two runs to compare
/compare?a=<run_id>&b=<run_id>  → side-by-side diff
```

---

## Currently supported languages

| Language | Extensions |
|---|---|
| C | `.c`, `.h` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx` |
| C# | `.cs` |
| Go | `.go` |
| Java | `.java` |
| JavaScript | `.js`, `.mjs`, `.cjs` |
| Python | `.py` |
| Rust | `.rs` |
| Shell | `.sh`, `.bash`, `.zsh`, `.ksh` |
| PowerShell | `.ps1`, `.psm1`, `.psd1` |
| TypeScript | `.ts`, `.mts`, `.cts` |

> **Note:** TOML, Markdown, and YAML are not analyzed (no meaningful SLOC metric applies). All languages above use a fast lexical state-machine parser. Python, C, and C++ will additionally gain tree-sitter-backed adapters for higher-accuracy parsing.

### Adding a new language

Adding language support requires changes in two crates:

1. **`crates/sloc-languages/src/lib.rs`** — add a variant to `Language`, implement `display_name`/`as_slug`/`from_name`, register file extensions in `detect_language`, and add a `ScanConfig` entry in `analyze_text`.
2. **`crates/sloc-config/src/lib.rs`** — add the language name to any allowlists used by `AnalysisConfig` if you want it on by default.

---

## PDF export

PDF generation uses a locally installed Chromium-based browser (Chrome, Edge, Brave, Vivaldi, or Opera). Generation runs in the background — the web UI returns results immediately while the PDF is being written.

oxide-sloc tries `--headless=old` first (required for newer Brave builds), then falls back to `--headless`.

If browser discovery fails, set the path manually:

```bash
export SLOC_BROWSER=/usr/bin/chromium
oxidesloc report result.json --pdf-out result.pdf
```

PDF downloads are named `<report-title>.pdf` rather than a generic filename.

In Docker, Chromium is bundled in the image — no extra setup needed.

---

## Export to CSV and Excel

Every HTML report includes **Export CSV** and **Export Excel** buttons in the top navigation bar, as well as in the "Per-file detail" table toolbar. Clicking either button downloads the per-file breakdown as a `.csv` or `.xls` file respectively — no server round-trip required, the export is generated entirely in the browser from the rendered table data.

---

## Git submodule support

Projects that use **git submodules** (a "super-repository" with dozens of nested sub-projects inside) can be analyzed with per-submodule isolation so each sub-project's SLOC totals are reported separately.

### How it works

1. oxide-sloc reads the `.gitmodules` file in the project root.
2. Each listed submodule path is used to tag every source file with its parent submodule.
3. The result page includes an extra **Submodule breakdown** table showing per-submodule file counts, code lines, comment lines, and physical lines.
4. Each submodule also gets its own linked HTML sub-report, saved alongside the main report.
5. The overall project totals still include all files — the submodule table is additive detail, not a replacement.

### CLI usage

```bash
oxidesloc analyze ./mono-repo \
  --submodule-breakdown \
  --json-out out/result.json \
  --html-out out/report.html
```

### Web UI

Enable the **Detect and separate git submodules** checkbox in Step 1 before running the scan. The result page will include a Submodule breakdown section with links to each sub-report.

### TOML config

```toml
[discovery]
submodule_breakdown = true
```

---

## Metrics API

When the web UI server is running, a JSON metrics API is available for CI/CD dashboards and custom tooling.

| Endpoint | Auth required | Description |
|---|---|---|
| `GET /api/metrics/latest` | Yes | Metrics for the most recent scan across all projects |
| `GET /api/metrics/:run_id` | Yes | Metrics for a specific run by its UUID |
| `GET /api/project-history?path=<dir>` | Yes | Scan history for a specific project root |
| `GET /badge/:metric` | No | SVG badge (shields.io-style) |
| `GET /embed/summary` | No | Embeddable HTML summary widget |
| `GET /healthz` | No | Health check — always returns `200 OK` |

### Metric values for `/badge/:metric`

| Metric | Description |
|---|---|
| `code-lines` | Total code lines in the latest scan |
| `files` | Total files analyzed |
| `comment-lines` | Total comment lines |
| `blank-lines` | Total blank lines |

Optional query parameters: `label=<override>` and `color=<hex>`.

```
# Example badge URLs
http://127.0.0.1:4317/badge/code-lines
http://127.0.0.1:4317/badge/code-lines?label=Source+Lines&color=d37a4c
```

Embed in a README:

```markdown
![Code Lines](http://your-host:4317/badge/code-lines)
```

### Embed widget

The `/embed/summary` endpoint returns a self-contained HTML snippet suitable for embedding in Confluence, Notion, or any tool that accepts `<iframe>` content:

```html
<iframe src="http://your-host:4317/embed/summary" width="100%" height="180" frameborder="0"></iframe>
```

---

## CI/CD

oxide-sloc ships ready-to-use pipeline files for Jenkins, GitHub Actions, and GitLab CI. No plugins or integrations are required — the `oxidesloc` binary is the only dependency beyond a standard Rust toolchain.

For detailed setup guides including Confluence publishing, see [`docs/ci-integrations.md`](./docs/ci-integrations.md).

### Web UI → CLI translation

Every web UI option maps 1:1 to a CLI flag, making it straightforward to reproduce any web-configured scan in a pipeline:

| Web UI step | CLI equivalent |
|---|---|
| Step 1: select project folder | `oxidesloc analyze ./my-repo` |
| Step 1: include pattern | `--include-glob "src/**"` |
| Step 1: exclude pattern | `--exclude-glob "vendor/**"` |
| Step 1: submodule breakdown | `--submodule-breakdown` |
| Quick Scan button | `oxidesloc analyze ./my-repo --plain` |
| Step 2: mixed-line policy | `--mixed-line-policy code-only` |
| Step 2: Python docstrings as code | `--python-docstrings-as-code` |
| Step 3: JSON output | `--json-out result.json` |
| Step 3: HTML output | `--html-out report.html` |
| Step 3: PDF output | `--pdf-out report.pdf` |
| Step 3: custom title | `--report-title "My Report"` |
| Re-render from saved JSON | `oxidesloc report result.json --html-out report.html` |
| Custom config file | `--config ci/sloc-ci-default.toml` |

### CI config presets

The `ci/` directory contains ready-to-use `sloc.toml` files for common pipeline scenarios:

| File | Use case |
|---|---|
| `ci/sloc-ci-default.toml` | Balanced defaults — mirrors web UI out of the box |
| `ci/sloc-ci-strict.toml` | Fail-fast — pipeline errors if binary files are found |
| `ci/sloc-ci-full-scope.toml` | Audit mode — counts everything including vendor/lockfiles |

```bash
# Use a preset in any pipeline stage
oxidesloc analyze ./src --config ci/sloc-ci-strict.toml \
  --json-out out/result.json \
  --html-out out/report.html
```

### GitHub Actions

Three workflows ship in `.github/workflows/`:

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to `main`, all PRs | fmt → clippy → build → unit tests → security audit → CLI smoke tests → web UI health check |
| `release.yml` | push a `v*` tag | cross-compile for 4 platforms → publish GitHub Release with binaries |
| `docker.yml` | push to `main`, push a `v*` tag | build and push Docker image to GHCR with `latest` + semver tags |

The `ci.yml` smoke job runs every analysis variant (plain, per-file, all 4 policies, JSON+HTML, re-render from JSON) and verifies the web UI responds HTTP 200.

To cut a release:

```bash
git tag v0.2.0
git push origin v0.2.0
```

Pushing a `v*` tag triggers both `release.yml` (binaries) and `docker.yml` (Docker image) automatically.

### Jenkins

A `Jenkinsfile` is included at the repo root. It auto-installs Rust on the agent if not present.

**Setup:**

1. Create a new **Pipeline** job in Jenkins.
2. Set **Definition** → `Pipeline script from SCM`.
3. Point it at this repository.
4. Jenkins will auto-discover the `Jenkinsfile`.

**Pipeline stages:**

```
Install Rust → Format → Lint → Unit tests → Build
  → Smoke: plain summary
  → Smoke: JSON + HTML reports
  → Smoke: per-file breakdown
  → Smoke: all 4 policy variants
  → Smoke: re-render from JSON
  → Smoke: HTML content sanity
  → Web UI health check
  → Archive binary + CI reports
```

**Environment variables:**

| Variable | Purpose |
|---|---|
| `RUST_LOG` | Tracing verbosity (`warn`, `info`, `debug`) |
| `SLOC_BROWSER` | Override Chromium path for PDF export (also checked: `BROWSER`) |
| `SLOC_API_KEY` | Enable API key auth — every request must carry `X-API-Key: <value>` |
| `SLOC_REGISTRY_PATH` | Override the scan-history registry location (default: `out/web/registry.json`) |
| `SKIP_WEB_CHECK` | Set to any non-empty value to skip the web UI health check stage |

### GitLab CI

`.gitlab-ci.yml` is included at the repo root. Push it to any GitLab project — the pipeline is auto-detected.

**Stages:** `quality` → `build` → `smoke` → `archive`

Smoke jobs run in parallel: `smoke:plain`, `smoke:per-file`, `smoke:reports`, `smoke:re-render`, `smoke:policies`, `smoke:web-ui`.

CI reports are uploaded as GitLab artifacts and retained for 7 days.

---

## Local development

### Prerequisites

- [Rust](https://rustup.rs) 1.78 or later
- `make` (Linux/macOS) — optional but recommended

### Make targets

```bash
make help         # list all targets

make check        # fmt + lint + test  ← run before every push
make dev          # fmt + lint + test + serve

make fmt          # cargo fmt --all
make lint         # cargo clippy -D warnings
make test         # cargo test --workspace
make build        # release binary → target/release/oxidesloc
make serve        # start web UI on http://127.0.0.1:4317
make analyze DIR=./my-repo   # CLI analyze

make docker-build # build Docker image locally
make docker-run   # run web UI in Docker on port 4317

make clean        # cargo clean
```

### Without make (Windows / raw commands)

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p oxidesloc -- serve
```

### Formatting

Configured in `rustfmt.toml`: `edition = "2021"`, `max_width = 100`.

### CI gates (must pass before merging)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

---

## Repository layout

```
.
├── crates/
│   ├── sloc-cli/         # CLI entry point and commands
│   ├── sloc-config/      # Config schema and TOML parsing
│   ├── sloc-core/        # File discovery, decoding, aggregation, JSON model, delta engine
│   ├── sloc-languages/   # Language detection and lexical analyzers
│   ├── sloc-report/      # HTML rendering (Askama), PDF export, CSV/Excel export
│   └── sloc-web/
│       ├── static/       # Bundled static assets (Chart.js — no CDN needed)
│       └── src/          # Axum web server, scan registry, metrics API, badge endpoint
├── dist/
│   ├── oxidesloc-windows-x64.zip        # Pre-built Windows binary (used by run.bat)
│   ├── oxidesloc-linux-x86_64.tar.gz    # Pre-built Linux binary — static musl (used by run.sh)
│   └── vendor-sources.7z                # Rust crate sources for air-gapped source builds
├── run.bat               # Windows zero-dep launcher — double-click or run from terminal
├── run.sh                # Linux zero-dep launcher — bash run.sh
├── vendor/               # All 328 Rust crate sources — enables offline builds
├── .cargo/
│   └── config.toml       # Tells Cargo to use vendor/ instead of crates.io
├── ci/
│   ├── sloc-ci-default.toml    # CI config preset — balanced defaults
│   ├── sloc-ci-strict.toml     # CI config preset — fail on binaries
│   └── sloc-ci-full-scope.toml # CI config preset — audit everything
├── .github/
│   └── workflows/
│       ├── ci.yml            # PR / push checks + smoke tests + security audit
│       ├── release.yml       # Cross-platform binary releases
│       ├── docker.yml        # Build and push Docker image to GHCR
│       └── update-dist.yml   # Rebuild and commit dist/ bundles (run manually or on tag)
├── docs/
│   ├── airgap.md             # Air-gapped / offline installation guide
│   ├── ci-integrations.md    # Jenkins, GitHub Actions, GitLab CI, Confluence
│   └── licensing-commercial.md  # Commercial / enterprise licensing info
├── samples/
│   └── basic/            # Fixture files used by CI smoke tests
├── Dockerfile
├── docker-compose.yml
├── Makefile
├── Jenkinsfile
├── .gitlab-ci.yml
├── sloc.example.toml
└── Cargo.toml
```

---

## Security

By default oxide-sloc binds to `127.0.0.1:4317` (localhost only). It can be deployed on a LAN or WLAN for personal or team use with the following measures.

### Hardened defaults

- HTTP request bodies are capped at 10 MB
- Error details are logged server-side only; generic messages are shown in the browser
- PDF generation uses Rust's `Command::args([...])` (no shell interpolation)
- Dependency CVEs are checked on every CI run via `cargo audit`

### LAN / team deployment

**Step 1 — bind to a network interface**

```bash
# Bind to all interfaces (or use a specific LAN IP)
oxidesloc serve --bind 0.0.0.0:4317
```

Or set it in `sloc.toml`:

```toml
[web]
bind_address = "0.0.0.0:4317"
```

**Step 2 — enable API key authentication**

Set `SLOC_API_KEY` in the server environment. When set, every request must carry a matching `X-API-Key` header. Requests without the correct key receive HTTP 401. The health check endpoint (`/healthz`) and the badge/embed endpoints are exempt so monitoring probes and external widgets continue to work.

```bash
export SLOC_API_KEY="$(openssl rand -hex 32)"
oxidesloc serve --bind 0.0.0.0:4317
```

**Step 3 — terminate TLS at a reverse proxy**

oxide-sloc speaks plain HTTP. Put it behind nginx, Caddy, or Traefik for HTTPS termination:

```nginx
server {
    listen 443 ssl;
    server_name sloc.internal;
    ssl_certificate     /etc/ssl/certs/sloc.crt;
    ssl_certificate_key /etc/ssl/private/sloc.key;

    location / {
        proxy_pass http://127.0.0.1:4317;
        proxy_set_header X-API-Key $http_x_api_key;
    }
}
```

To report a vulnerability privately, see [`SECURITY.md`](./SECURITY.md).

---

## License

**oxide-sloc** is licensed under [AGPL-3.0-or-later](./LICENSE).
Copyright (C) 2026 Nima Shafie. All intellectual property rights vest solely in the author.

Third-party dependencies are distributed under their own licenses; see `Cargo.lock` and each crate's license metadata for details.

Commercial support, hosted services, and proprietary add-ons are available through separate arrangements. See [`docs/licensing-commercial.md`](./docs/licensing-commercial.md).

---

## Maintainer

**Nima Shafie** — [github.com/NimaShafie](https://github.com/NimaShafie)
