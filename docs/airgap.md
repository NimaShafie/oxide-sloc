# Air-Gap / Offline Deployment

## TL;DR

**Windows 10/11** — double-click `run.bat`

**Linux (RHEL 8/9, Ubuntu 18+, Debian 10+, any x86-64)**

```bash
bash run.bat
```

That is all. The script finds the pre-built binary in `dist/`, extracts it, and starts
the web UI at **http://127.0.0.1:4317**.

No Rust. No package manager. No internet. No extra tools.

---

## What each script requires

| Platform | Script | Required tools |
|---|---|---|
| Windows 10/11 | `run.bat` | PowerShell 5+ (built into every Windows 10/11 install) |
| Linux x86-64 | `bash run.bat` | `bash` + `tar` (present on every RHEL/Ubuntu/Debian install) |

---

## Step-by-step: Windows

1. Transfer this repository folder (including `dist/`) to the target machine via USB, file
   share, or internal network.
2. Open the folder in Explorer and double-click **`run.bat`** — or open a terminal and run:
   ```
   run.bat
   ```
3. `run.bat` extracts `dist\oxidesloc-windows-x64.zip` using PowerShell's built-in
   `Expand-Archive` and launches the binary. A terminal window opens showing the server
   output. The web UI is available at **http://127.0.0.1:4317**.

---

## Step-by-step: Linux (RHEL 8/9)

1. Transfer this repository folder (including `dist/`) to the target machine.
2. Open a terminal in the repository root and run:
   ```bash
   bash run.bat
   ```
3. `run.bat` extracts `dist/oxidesloc-linux-x86_64.tar.gz` using `tar` and starts the
   server. Open **http://127.0.0.1:4317** in a browser. Press `Ctrl+C` to stop.

---

## What is bundled in `dist/`

| File | Platform | Notes |
|---|---|---|
| `oxidesloc-windows-x64.zip` | Windows x86-64 | Extracted by `run.bat` via PowerShell |
| `oxidesloc-linux-x86_64.tar.gz` | Linux x86-64 | Extracted by `run.bat` via `tar`. Static musl build — no glibc dependency, runs on RHEL 8+ |
| `vendor-sources.7z` | All | Rust crate sources for building from source (optional path only) |

The Linux binary is built with `x86_64-unknown-linux-musl` (fully static). It carries zero
runtime library dependencies and runs on RHEL 8 (glibc 2.28), RHEL 9 (glibc 2.34), and
any other x86-64 Linux.

---

## Keeping dist/ current (maintainer task)

When a new version ships, the `dist/` bundles need to be regenerated. Run the
**Update dist bundles** workflow from the GitHub Actions tab (no inputs needed), or push
a `v*` tag which triggers it automatically.

The workflow builds both platform binaries, packages them, and commits the result to
`dist/` in one step.

---

## Fallback: build from source

Use this path only when you need to build from source on the air-gapped machine itself
(e.g. unsupported architecture, custom patches).

The `vendor/` directory contains all 328 Rust crate dependencies. Cargo reads from it
automatically via `.cargo/config.toml` — no internet access needed after cloning.

```bash
# The machine must have Rust installed (see below if it does not)
cargo build --release --workspace --offline

# Binary output:
#   target/release/oxidesloc        (Linux / macOS)
#   target\release\oxidesloc.exe   (Windows)

# Start the web UI
./target/release/oxidesloc serve
# or just: bash run.bat    (the script finds target/release/ automatically)
```

### Bundling a Rust toolchain for a machine with no internet and no Rust

On a networked machine, archive the toolchain and bring it along:

**Windows**
```powershell
# Install rustup once, then archive
rustup-init.exe --default-toolchain stable --no-modify-path
Compress-Archive -Path "$env:USERPROFILE\.rustup","$env:USERPROFILE\.cargo" `
    -DestinationPath rust-toolchain-windows.zip
```

**Linux**
```bash
./rustup-init --default-toolchain stable --no-modify-path
tar -czf rust-toolchain-linux.tar.gz ~/.rustup ~/.cargo
```

On the air-gapped machine, extract the archive to the same paths, add
`~/.cargo/bin` (or `%USERPROFILE%\.cargo\bin`) to `PATH`, and run
`cargo build --release --workspace --offline`.

---

## Runtime network requirements

| Feature | Network needed? |
|---|---|
| `serve` command (web UI) | No |
| `analyze` command | No |
| `report` command | No |
| PDF export | No — uses locally installed Chromium |
| `--smtp-to` email delivery | Yes |
| `--webhook-url` delivery | Yes |

PDF export requires a locally installed Chromium-based browser (Chrome, Edge, Brave,
Vivaldi, or Opera). Set `SLOC_BROWSER=/path/to/chromium` if auto-discovery fails.
