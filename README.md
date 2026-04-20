# oxide-sloc

[![CI](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml)
[![Release](https://github.com/NimaShafie/oxide-sloc/actions/workflows/release.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/release.yml)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](./LICENSE)

**oxide-sloc** is a Rust-based source line analysis tool built for teams that want more than a simple line counter.

One shared analysis core with multiple delivery surfaces:

- **CLI** — `oxidesloc analyze / report / serve`
- **Localhost web UI** — guided 4-step flow with light/dark theme, auto browser-open
- **Rich HTML reports** — per-file breakdown, language summaries, warning analysis
- **PDF export** — non-blocking background generation via locally installed Chromium
- **Policy-aware counting** — mixed code/comment lines, Python docstrings
- **CI/CD ready** — Jenkinsfile, GitHub Actions, and GitLab CI pipelines included

---

## Installation

### Option 1 — Pre-built binary (no Rust required)

Download the latest binary for your platform from the [Releases page](https://github.com/NimaShafie/oxide-sloc/releases):

| Platform | File |
|---|---|
| Linux x86-64 | `oxidesloc-linux-x86_64` |
| Windows x86-64 | `oxidesloc-windows-x86_64.exe` |
| macOS x86-64 | `oxidesloc-macos-x86_64` |
| macOS Apple Silicon | `oxidesloc-macos-arm64` |

```bash
# Linux / macOS — make executable and move to PATH
chmod +x oxidesloc-linux-x86_64
mv oxidesloc-linux-x86_64 /usr/local/bin/oxidesloc

# Windows — rename and add to PATH
ren oxidesloc-windows-x86_64.exe oxidesloc.exe
```

### Option 2 — Docker (no Rust required)

```bash
# Build locally and run the web UI
docker compose up
```

Open [http://localhost:4317](http://localhost:4317) in your browser.

> **Note:** The first `docker compose up` will build the image, which takes a few minutes. Subsequent runs start instantly.

To analyze a directory from the CLI via Docker:

```bash
docker run --rm \
  -v /path/to/your/repo:/repo:ro \
  oxide-sloc \
  analyze /repo --plain
```

Chromium is bundled in the Docker image — PDF export works out of the box.

### Option 3 — Build from source (requires Rust 1.78+)

```bash
cargo install --path crates/sloc-cli
```

Or build without installing:

```bash
cargo build --release -p oxidesloc
./target/release/oxidesloc --help
```

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
| `--config` | path | `sloc.toml` | Load settings from TOML config file |

### Web UI

```bash
oxidesloc serve
# → http://127.0.0.1:4317  (opens automatically)
```

The web UI is a guided 4-step flow:

| Step | What it configures |
|---|---|
| **1 — Select project** | Target folder, include/exclude glob patterns, live scope preview |
| **2 — Counting rules** | Mixed-line policy, Python docstring handling, generated/minified/vendor/lockfile/binary file behavior |
| **3 — Outputs and reports** | Scan preset, artifact preset, output directory, report title |
| **4 — Review and run** | Summary of all settings, one-click scan |

Everything available in the web UI maps directly to a CLI flag — see [Web UI → CLI translation](#web-ui--cli-translation).

### Configuration file

Copy the example config and edit it:

```bash
cp sloc.example.toml sloc.toml
```

CLI flags always override config file values. Run `oxidesloc --help` for the full flag list.

---

## Currently supported languages

| Language | Extensions |
|---|---|
| C | `.c`, `.h` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp` |
| C# | `.cs` |
| Python | `.py` |
| Shell | `.sh`, `.bash`, `.zsh`, `.ksh` |
| PowerShell | `.ps1`, `.psm1`, `.psd1` |

> **Note:** Rust, TOML, Markdown, and YAML are not yet supported. Running oxide-sloc against its own repository will skip most files.

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

## CI/CD

oxide-sloc ships ready-to-use pipeline files for Jenkins, GitHub Actions, and GitLab CI. No plugins or integrations are required — the `oxidesloc` binary is the only dependency beyond a standard Rust toolchain.

### Web UI → CLI translation

Every web UI option maps 1:1 to a CLI flag, making it straightforward to reproduce any web-configured scan in a pipeline:

| Web UI step | CLI equivalent |
|---|---|
| Step 1: select project folder | `oxidesloc analyze ./my-repo` |
| Step 1: include pattern | `--include-glob "src/**"` |
| Step 1: exclude pattern | `--exclude-glob "vendor/**"` |
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

Two workflows ship in `.github/workflows/`:

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to `main`, all PRs | fmt → clippy → build → unit tests → CLI smoke tests → web UI health check |
| `release.yml` | push a `v*` tag | cross-compile for 4 platforms → publish GitHub Release with binaries |

The `ci.yml` smoke job runs every analysis variant (plain, per-file, all 4 policies, JSON+HTML, re-render from JSON) and verifies the web UI responds HTTP 200.

To cut a release:

```bash
git tag v0.2.0
git push origin v0.2.0
```

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
| `SLOC_BROWSER` | Override Chromium path for PDF export |
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

---

## Repository layout

```
.
├── crates/
│   ├── sloc-cli/         # CLI entry point and commands
│   ├── sloc-config/      # Config schema and TOML parsing
│   ├── sloc-core/        # File discovery, decoding, aggregation, JSON model
│   ├── sloc-languages/   # Language detection and lexical analyzers
│   ├── sloc-report/      # HTML rendering and PDF export
│   └── sloc-web/         # Localhost web UI (Axum)
├── ci/
│   ├── sloc-ci-default.toml    # CI config preset — balanced defaults
│   ├── sloc-ci-strict.toml     # CI config preset — fail on binaries
│   └── sloc-ci-full-scope.toml # CI config preset — audit everything
├── .github/
│   └── workflows/
│       ├── ci.yml        # PR / push checks + smoke tests
│       └── release.yml   # Cross-platform binary releases
├── docs/
│   ├── licensing.md
│   └── licensing-commercial.md
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

oxide-sloc is designed as a **localhost-only tool** — the web UI binds to `127.0.0.1:4317` and is not intended to be exposed to a network.

- HTTP request bodies are capped at 10 MB
- Error details are logged server-side only; generic messages are shown in the browser
- PDF generation uses Rust's `Command::args([...])` (no shell interpolation)
- Dependency CVEs are checked on every CI run via `cargo audit`

To report a vulnerability privately, see [`SECURITY.md`](./SECURITY.md).

---

## Roadmap

1. Add tree-sitter-backed adapters (Python and C/C++ first)
2. Add validation corpus and golden tests
3. Add SMTP and webhook delivery (`send` command)
4. Publish Docker image to GitHub Container Registry
5. Expand supported languages (Rust, Go, Java, TypeScript)

---

## License

[AGPL-3.0-or-later](./LICENSE). Commercial support, hosted services, and proprietary add-ons are available through separate arrangements. See [`docs/licensing.md`](./docs/licensing.md) and [`docs/licensing-commercial.md`](./docs/licensing-commercial.md).

---

## Maintainer

**Nima Shafie** — [github.com/NimaShafie](https://github.com/NimaShafie)
