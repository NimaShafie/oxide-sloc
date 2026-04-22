# oxide-sloc — Air-Gap Installation Guide

Everything needed to build and run oxide-sloc without internet access is included in this repository:

| What | How it's bundled |
|------|-----------------|
| Rust crate dependencies (328 crates) | `vendor/` directory — Cargo reads from it automatically |
| Chart.js 4.4.0 (web UI chart library) | Compiled into the binary via `include_bytes!` — no CDN call at runtime |
| Analysis core, web server, HTML/PDF reports | All Rust source, builds locally |

---

## Prerequisites

You need the **Rust compiler** (`rustc` + `cargo`) installed on the target machine. The vendored sources let you build without crates.io, but the compiler itself must be present.

### Option A — Pre-built binary (no Rust needed)

Download a release binary from the GitHub releases page on a networked machine, transfer it via USB/internal file share, and run it directly. No build step required.

```
oxidesloc.exe   (Windows x86-64)
oxidesloc        (Linux x86-64 / musl)
```

### Option B — Build from vendored sources

**Step 1 — Get Rust on the air-gapped machine**

On a networked machine, download the Rust toolchain installer and the stable toolchain archive for your target platform:

```bash
# Windows
# Download: https://win.rustup.rs/x86_64  (rustup-init.exe)
# Then, with internet available, pre-fetch the toolchain:
rustup toolchain add stable-x86_64-pc-windows-msvc
# Copy %USERPROFILE%\.rustup to the air-gapped machine

# Linux x86-64
# Download: https://sh.rustup.rs  (rustup-init)
# Pre-fetch: rustup toolchain add stable-x86_64-unknown-linux-gnu
# Copy ~/.rustup to the air-gapped machine
```

On the air-gapped machine, run `rustup-init` pointing at the local cache, or just put `~/.rustup/toolchains/stable-*/bin` on your `PATH`.

**Step 2 — Clone / copy this repo**

Transfer the entire repository (including the `vendor/` directory) to the air-gapped machine. All 328 crate sources are in `vendor/`.

**Step 3 — Build**

```bash
# .cargo/config.toml already configures Cargo to use vendor/
cargo build --release --workspace

# The binary is at:
#   target/release/oxidesloc        (Linux)
#   target\release\oxidesloc.exe   (Windows)
```

No network access is required. Cargo reads all dependencies from `vendor/`.

---

## Verifying the build is fully offline

After build, the binary is self-contained:

```bash
# Confirm no CDN calls when the web UI loads
cargo run -p oxidesloc -- serve
# Open http://localhost:3000 — Chart.js loads from /static/chart.js (served by the binary itself)
```

---

## Runtime network requirements

| Feature | Network required? |
|---------|------------------|
| `analyze` command (local scan) | No |
| `report` command (HTML/PDF from JSON) | No |
| `serve` command (web UI) | No — Chart.js is bundled |
| PDF export | No — uses locally installed Chromium |
| `--smtp-to` (email delivery) | Yes — connects to configured SMTP host |
| `--webhook-url` (webhook POST) | Yes — connects to configured URL |

SMTP and webhook delivery are optional. Do not pass `--smtp-to` or `--webhook-url` on air-gapped machines.

---

## Keeping vendored sources current

When updating dependencies (e.g. after `cargo update`):

```bash
# Run once on a networked machine, then commit the result
cargo vendor vendor
git add vendor/ .cargo/config.toml
git commit -m "chore: refresh vendored dependencies"
```

---

## PDF export on air-gapped systems

PDF generation invokes a locally installed Chromium-based browser in headless mode. Install one of the following **before** the network is removed:

- Google Chrome
- Microsoft Edge
- Brave Browser
- Vivaldi
- Opera

Or set `SLOC_BROWSER=/path/to/chromium` to point at any Chromium binary.
