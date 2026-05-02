# Air-Gap / Offline Deployment

## TL;DR

Run the install script once, then launch with `bash scripts/run.sh`.

| Platform | Install | Launch |
|---|---|---|
| **Windows 10/11** | `bash scripts/install.sh` (in Git Bash) | `bash scripts/run.sh` (in Git Bash) |
| **Linux (RHEL 8/9, Ubuntu, Debian)** | `bash scripts/install.sh` | `bash scripts/run.sh` |

No internet. No package manager. No extra tools beyond what ships with the OS.

---

## What `scripts/install.sh` does

The install script tries each path in order and stops at the first success:

1. **Binary already present** — `oxide-sloc.exe` / `oxide-sloc` is in the repo root → nothing to do.
2. **Pre-built binary in `dist/`** — extracts `dist/oxide-sloc-windows-x64.zip` (Windows, via built-in PowerShell) or `dist/oxide-sloc-linux-x86_64.tar.gz` (Linux, via `tar`). No extra tools needed.
3. **Rust installed + vendor archive present** — decompresses `vendor.tar.xz` to `vendor/` if not already present, writes `.cargo/config.toml` to redirect cargo offline, then runs `cargo build --release --offline`. All crate dependencies are in the archive; no internet access required.
4. **Nothing works** — prints clear instructions for bundling the Rust toolchain on a networked machine and transferring it.

> **What vendor.tar.xz covers:** All Rust crate source dependencies (~328 crates). It does **not** include the Rust toolchain itself (rustc, cargo). See [below](#building-from-source-on-a-machine-with-no-rust-and-no-internet) if you also need to transfer the toolchain.

---

## Required tools per path

| Path | Windows | Linux |
|---|---|---|
| Pre-built binary | PowerShell 5+ (built into Windows 10/11) | `bash` + `tar` (present on every RHEL/Ubuntu install) |
| Source build | `cargo` (Rust toolchain) + `vendor.tar.xz` | `cargo` (Rust toolchain) + `vendor.tar.xz` |

---

## Transferring to an air-gapped machine

### Small transfer — binary only (~5 MB)

Download the pre-built binary for your platform from the [GitHub releases page](https://github.com/oxide-sloc/oxide-sloc/releases) and add it to the transfer bundle alongside the scripts.

**Windows (PowerShell):**
```powershell
# Download oxide-sloc-windows-x86_64.exe from the release page first, then:
Compress-Archive -Path scripts, dist -DestinationPath oxide-sloc-deploy.zip
```

**Linux:**
```bash
# Download oxide-sloc-linux-x86_64 from the release page first, then:
tar -czf oxide-sloc-deploy.tar.gz scripts/ dist/
```

Extract on the target machine and run `bash scripts/install.sh`.

### Full transfer — source build bundle (~200 MB, includes vendored Rust dependencies)

Download `vendor.tar.xz` and `vendor.tar.xz.sha256` from the same [GitHub release page](https://github.com/oxide-sloc/oxide-sloc/releases) as the binaries. Then bundle with the source tree:

**Linux:**
```bash
# After downloading vendor.tar.xz to the repo root:
make bundle
# Produces: oxide-sloc-bundle.tar.gz
```

**Windows (PowerShell):**
```powershell
Compress-Archive -Path scripts, dist, vendor.tar.xz, vendor.tar.xz.sha256 `
    -DestinationPath oxide-sloc-bundle.zip
```

Transfer the archive to the target machine, extract it, and run `bash scripts/install.sh`.

> **Why ~200 MB?** `vendor.tar.xz` (~27 MB) contains all Rust crate sources compressed at xz-extreme ratio. The `target/` compiled artifacts (4+ GB) are excluded — platform-specific and rebuilt locally. `scripts/install.sh` decompresses `vendor.tar.xz` to `vendor/` automatically before building.

### Quickest source-only build (advanced)

For developers who want just the source build without the install script:

```bash
# On the air-gapped machine (Rust toolchain must already be installed):
bash scripts/airgap-build.sh vendor.tar.xz
# Binary lands at: target/release/oxide-sloc
```

---

## Building from source on a machine with no Rust and no internet

You need to pre-package the Rust toolchain on a networked machine and carry it over.

### Bundle the toolchain (do this on a networked machine)

**Windows:**
```powershell
rustup-init.exe --default-toolchain stable --no-modify-path
Compress-Archive -Path "$env:USERPROFILE\.rustup","$env:USERPROFILE\.cargo" `
    -DestinationPath rust-toolchain-windows.zip
```

**Linux:**
```bash
curl -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable --no-modify-path
tar -czf rust-toolchain-linux.tar.gz ~/.rustup ~/.cargo
```

### Restore on the air-gapped machine

**Windows:**
```powershell
Expand-Archive rust-toolchain-windows.zip -DestinationPath $env:USERPROFILE
# Add to PATH (run once, then reopen terminal):
[Environment]::SetEnvironmentVariable("PATH", "$env:USERPROFILE\.cargo\bin;" + $env:PATH, "User")
```

**Linux:**
```bash
tar xzf rust-toolchain-linux.tar.gz -C ~
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

Then run `bash scripts/install.sh` — it locates `vendor.tar.xz`, extracts it, and builds from `vendor/` automatically.

---

## CI/CD on air-gapped infrastructure

### Jenkins

The included `Jenkinsfile` auto-installs Rust on the agent if not present. For a fully offline agent, pre-install the toolchain using the steps above, then place `vendor.tar.xz` in the workspace. The pipeline decompresses it once and caches `vendor/` between runs — no outbound network traffic during `cargo build`.

### GitLab CI

The included `.gitlab-ci.yml` works the same way. Use a self-hosted GitLab runner with Rust pre-installed. Place `vendor.tar.xz` in the runner's workspace cache or provide it via a custom pre-clone step.

### GitHub Actions (internal/self-hosted runner)

Use the standard `ci.yml` workflow on a self-hosted runner. For internet-connected runners, cargo downloads crates normally via Swatinem/rust-cache. For air-gapped runners, place `vendor.tar.xz` in the workspace and re-add the vendor extraction step to the workflow.

---

## Runtime network requirements

| Feature | Network needed? |
|---|---|
| Web UI (`serve`) | No |
| `analyze` command | No |
| `report` command | No |
| PDF export | No — uses locally installed Chromium |
| Email delivery (`--smtp-to`) | Yes |
| Webhook delivery (`--webhook-url`) | Yes |

PDF export requires a locally installed Chromium-based browser (Chrome, Edge, Brave, Vivaldi, or Opera). Set `SLOC_BROWSER=/path/to/chromium` if auto-discovery fails.
