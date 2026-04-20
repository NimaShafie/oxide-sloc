# oxide-sloc

[![CI](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/ci.yml)
[![Release](https://github.com/NimaShafie/oxide-sloc/actions/workflows/release.yml/badge.svg)](https://github.com/NimaShafie/oxide-sloc/actions/workflows/release.yml)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](./LICENSE)

**oxide-sloc** is a Rust-based source line analysis tool built for teams that want more than a simple line counter.

One shared analysis core with multiple delivery surfaces:

- **CLI** — `oxidesloc analyze / report / serve`
- **Localhost web UI** — guided multi-step flow with light/dark theme
- **Rich HTML reports** — per-file breakdown, language summaries, warnings
- **PDF export** — via any locally installed Chromium-based browser
- **Policy-aware counting** — mixed code/comment lines, Python docstrings

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
# Pull and run the web UI
docker pull ghcr.io/nimashafie/oxide-sloc:latest
docker run -p 3000:3000 ghcr.io/nimashafie/oxide-sloc:latest

# Or build locally and run
docker compose up
```

Open [http://localhost:3000](http://localhost:3000) in your browser.

To analyze a directory from the CLI via Docker:

```bash
docker run --rm \
  -v /path/to/your/repo:/repo:ro \
  ghcr.io/nimashafie/oxide-sloc:latest \
  analyze /repo --plain
```

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
# Analyze a directory, print summary to terminal
oxidesloc analyze ./my-repo --plain

# Full output: JSON + HTML report
oxidesloc analyze ./my-repo \
  --json-out result.json \
  --html-out result.html

# Per-file breakdown
oxidesloc analyze ./my-repo --per-file

# Re-render a report from a saved JSON (useful for changing output format)
oxidesloc report result.json --pdf-out result.pdf

# Start the web UI
oxidesloc serve
```

### Web UI

```bash
oxidesloc serve
# → http://localhost:3000
```

The web UI walks through directory selection, analysis options, and report generation in a guided flow.

### Configuration

Copy the example config and edit it:

```bash
cp sloc.example.toml sloc.toml
```

All CLI flags override the config file. Run `oxidesloc --help` for the full flag list.

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

PDF generation requires a locally installed Chromium-based browser (Chrome, Edge, Brave, Vivaldi, or Opera).

If browser discovery fails, set the path manually:

```bash
export SLOC_BROWSER=/usr/bin/chromium
oxidesloc report result.json --pdf-out result.pdf
```

In Docker, Chromium is bundled in the image — no extra setup needed.

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
make serve        # start web UI on http://localhost:3000
make analyze DIR=./my-repo   # CLI analyze

make docker-build # build Docker image locally
make docker-run   # run web UI in Docker on port 3000

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

## CI/CD

### GitHub Actions

Two workflows ship in `.github/workflows/`:

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to `main`, all PRs | fmt check → clippy → build → test |
| `release.yml` | push a `v*` tag | cross-compile for 4 platforms → publish GitHub Release |

To cut a release:

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions builds binaries for Linux, Windows, macOS x86-64, and macOS ARM, then creates a release with auto-generated notes.

### Jenkins

A `Jenkinsfile` is included at the repo root. It runs the same gates as the GitHub Actions CI workflow and archives the release binary as a Jenkins artifact.

**Setup:**

1. Create a new **Pipeline** job in Jenkins.
2. Set **Definition** → `Pipeline script from SCM`.
3. Point it at this repository.
4. Jenkins will auto-discover the `Jenkinsfile`.

The pipeline auto-installs Rust via `rustup` on the agent if it is not already present — no pre-configuration of the Jenkins node is required.

**Stages:**

```
Install Rust → Format → Lint → Test → Build → Archive binary
```

**Environment variables you can set in Jenkins:**

| Variable | Purpose |
|---|---|
| `RUST_LOG` | Tracing verbosity (`info`, `debug`) |
| `SLOC_BROWSER` | Path to Chromium binary for PDF export |

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
├── .github/
│   └── workflows/
│       ├── ci.yml        # PR / push checks
│       └── release.yml   # Cross-platform binary releases
├── docs/
│   └── licensing.md
├── Dockerfile
├── docker-compose.yml
├── Makefile
├── Jenkinsfile
├── sloc.example.toml
└── Cargo.toml
```

---

## Near-term roadmap

1. Fix PDF generation in the web UI
2. Add tree-sitter-backed adapters (Python and C/C++ first)
3. Add validation corpus and golden tests
4. Add SMTP and webhook delivery (`send` command)
5. Publish Docker image to GitHub Container Registry

---

## License

[AGPL-3.0-or-later](./LICENSE). Commercial support, hosted services, and proprietary add-ons are available through separate arrangements. See `docs/licensing.md`.

---

## Maintainer

**Nima Shafie** — [github.com/NimaShafie](https://github.com/NimaShafie)
