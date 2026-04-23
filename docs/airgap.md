# oxide-sloc — Air-Gap / Offline Installation Guide

Everything needed to **build and run** oxide-sloc without internet access is included in
this repository. The table below shows exactly what is bundled and what (if anything) must
be staged before the network is removed.

| What | Status | How it's bundled |
|------|--------|-----------------|
| Rust crate dependencies (328 crates) | ✅ Bundled | `vendor/` directory — Cargo reads from it automatically via `.cargo/config.toml` |
| Chart.js 4.4.0 (web UI chart library) | ✅ Bundled | Compiled into the binary via `include_bytes!` — no CDN call at runtime |
| Analysis core, web server, HTML/PDF reports | ✅ Bundled | All Rust source, builds locally |
| Rust compiler (`rustc` + `cargo`) | ⚠️ Not in repo | Must be present on the build machine — see options below |

Choose the path that fits your environment:

| Path | Network ever needed? | Build step? |
|------|---------------------|-------------|
| [A — Pre-built binary](#option-a--pre-built-binary-completely-offline-no-rust-required) | No | No |
| [B — Source build, Rust already installed](#option-b--source-build-rust-already-on-the-machine) | No (after clone) | Yes |
| [C — Source build, bundle Rust toolchain for a fresh machine](#option-c--source-build-bundle-the-rust-toolchain-for-a-fresh-air-gapped-machine) | No (on the target) | Yes |

---

## Option A — Pre-built binary (completely offline, no Rust required)

The simplest path. Download a release binary on any machine that has internet access,
transfer it to the air-gapped machine via USB or internal file share, and run it directly.
No compiler, no build step, no network access on the target machine.

```
oxide-sloc-windows-x86_64.exe   (Windows x86-64)
oxide-sloc-linux-x86_64          (Linux x86-64, statically linked musl build)
```

Download from the [GitHub releases page](https://github.com/NimaShafie/oxide-sloc/releases).

```bash
# Linux — make executable and move to PATH
chmod +x oxide-sloc-linux-x86_64
mv oxide-sloc-linux-x86_64 /usr/local/bin/oxidesloc

# Windows — rename and add to PATH
ren oxide-sloc-windows-x86_64.exe oxidesloc.exe
```

That is all. The binary is self-contained: the web UI, all HTML/CSS/JS, and Chart.js are
compiled in. No external files are needed at runtime.

---

## Option B — Source build, Rust already on the machine

Use this path when the target machine already has Rust installed (common in development
environments and CI agents). The `vendor/` directory contains all 328 crate dependencies,
so `cargo build` requires no internet access after the repo is cloned.

**Step 1 — Transfer the repository**

Clone or copy this repository (including `vendor/`) to the target machine. If transferring
via USB or archive:

```bash
# On the machine with git access
git clone https://github.com/NimaShafie/oxide-sloc.git
cd oxide-sloc
# Then copy/zip the entire directory including vendor/
```

**Step 2 — Build and run**

```bash
# .cargo/config.toml already tells Cargo to read from vendor/ — no extra flags needed
cargo build --release --workspace

# Binary locations:
#   target/release/oxidesloc        (Linux / macOS)
#   target\release\oxidesloc.exe   (Windows)

# Verify no network was used:
cargo build --release --workspace --offline

# Start the web UI
./target/release/oxidesloc serve

# Or run directly without installing:
cargo run -p oxidesloc -- serve
```

No network access is required. Cargo reads all dependencies from `vendor/`.

---

## Option C — Source build, bundle the Rust toolchain for a fresh air-gapped machine

Use this path when the target machine has **no internet access and no Rust toolchain
installed**. The approach is to pre-stage the Rust toolchain on a networked machine,
then transfer the toolchain alongside the repository.

### Step 1 — Pre-stage the toolchain on a networked machine

**Windows (PowerShell or Git Bash):**

```bash
# Download the rustup installer
# https://win.rustup.rs/x86_64 → rustup-init.exe
# Run it once to install rustup and the stable toolchain:
rustup-init.exe --default-toolchain stable --no-modify-path

# The toolchain is now in %USERPROFILE%\.rustup and %USERPROFILE%\.cargo
# Verify the toolchain is fully downloaded:
rustup show
```

**Linux x86-64:**

```bash
# Download the rustup init script
# https://sh.rustup.rs → rustup-init
chmod +x rustup-init
./rustup-init --default-toolchain stable --no-modify-path

# The toolchain is now in ~/.rustup and ~/.cargo
rustup show
```

### Step 2 — Archive the toolchain and repository together

```bash
# On the networked machine — create a single transfer archive:
#
# Windows (PowerShell):
Compress-Archive -Path $env:USERPROFILE\.rustup, $env:USERPROFILE\.cargo, C:\path\to\oxide-sloc `
    -DestinationPath oxide-sloc-offline-bundle.zip
#
# Linux:
tar -czf oxide-sloc-offline-bundle.tar.gz \
    ~/.rustup \
    ~/.cargo \
    /path/to/oxide-sloc
```

Transfer `oxide-sloc-offline-bundle.zip` / `.tar.gz` to the air-gapped machine via USB
or internal file share.

### Step 3 — Restore and build on the air-gapped machine

**Windows:**

```bash
# Extract the archive to the same paths:
Expand-Archive oxide-sloc-offline-bundle.zip -DestinationPath C:\

# Set environment variables so Cargo finds the pre-staged toolchain
$env:RUSTUP_HOME = "$env:USERPROFILE\.rustup"
$env:CARGO_HOME  = "$env:USERPROFILE\.cargo"
$env:PATH        = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# Navigate to the repo and build
cd C:\oxide-sloc
cargo build --release --workspace --offline
```

**Linux:**

```bash
# Extract the archive
tar -xzf oxide-sloc-offline-bundle.tar.gz -C /

# Source the Cargo environment (or add to ~/.bashrc)
export RUSTUP_HOME="$HOME/.rustup"
export CARGO_HOME="$HOME/.cargo"
export PATH="$HOME/.cargo/bin:$PATH"

# Navigate to the repo and build
cd /path/to/oxide-sloc
cargo build --release --workspace --offline
```

### Alternative: separate toolchain archive

If you prefer to keep the toolchain and repo separate (e.g. one shared toolchain for
multiple projects):

```bash
# Windows — just archive the rustup directories
Compress-Archive -Path $env:USERPROFILE\.rustup, $env:USERPROFILE\.cargo `
    -DestinationPath rust-stable-toolchain-windows.zip

# Linux
tar -czf rust-stable-toolchain-linux.tar.gz ~/.rustup ~/.cargo
```

Transfer the toolchain archive and this repository separately, then restore as shown
in Step 3 above.

---

## Confirming the build used no network

After building, verify that every dependency was resolved from `vendor/`:

```bash
# Explicit offline flag — Cargo errors if anything tries to reach crates.io
cargo build --release --workspace --offline

# Start the web UI and confirm Chart.js loads from the bundled asset (not a CDN)
./target/release/oxidesloc serve
# Open http://127.0.0.1:4317 — view page source, confirm no external script tags
```

---

## Runtime network requirements

Once built, the binary itself requires no network access for any core feature:

| Feature | Network required? |
|---------|------------------|
| `analyze` command (local scan) | No |
| `report` command (HTML/PDF from JSON) | No |
| `serve` command (web UI) | No — Chart.js is compiled into the binary |
| PDF export | No — uses locally installed Chromium |
| `--smtp-to` (email delivery) | Yes — connects to your configured SMTP host |
| `--webhook-url` (webhook POST) | Yes — connects to your configured URL |

SMTP and webhook delivery are optional. Do not pass `--smtp-to` or `--webhook-url` on
air-gapped machines.

---

## PDF export on air-gapped systems

PDF generation invokes a locally installed Chromium-based browser in headless mode.
Install one of the following **before** the network is removed:

- Google Chrome
- Microsoft Edge
- Brave Browser
- Vivaldi
- Opera

Set `SLOC_BROWSER=/path/to/chromium` to point at any Chromium binary if auto-discovery
fails.

---

## Keeping vendored sources current

When updating Rust dependencies (`cargo update`) on a networked machine:

```bash
# Re-vendor all crates, then commit the result
cargo vendor vendor
git add vendor/ .cargo/config.toml
git commit -m "chore: refresh vendored dependencies"
```

The `.cargo/config.toml` at the repo root is pre-configured to point at `vendor/`;
no extra Cargo flags are needed after committing.
