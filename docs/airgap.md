# Air-Gap / Offline Deployment

## TL;DR

Run the install script once, then launch with `bash run.sh`.

| Platform | Install | Launch |
|---|---|---|
| **Windows 10/11** | `bash install.sh` (in Git Bash) | `bash run.sh` (in Git Bash) |
| **Linux (RHEL 8/9, Ubuntu, Debian)** | `bash install.sh` | `bash run.sh` |

No internet. No package manager. No extra tools beyond what ships with the OS.

---

## What `install.sh` does

The install script tries each path in order and stops at the first success:

1. **Binary already present** — `oxide-sloc.exe` / `oxide-sloc` is next to the script → nothing to do.
2. **Pre-built binary in `dist/`** — extracts `dist/oxide-sloc-windows-x64.zip` (Windows, via built-in PowerShell) or `dist/oxide-sloc-linux-x86_64.tar.gz` (Linux, via `tar`). No extra tools needed.
3. **Rust installed** — decompresses `vendor.tar.xz` (22 MB) to `vendor/` if not already present, then runs `cargo build --release --offline`. All 328 crate dependencies are in the archive; no internet access required.
4. **Nothing works** — prints clear instructions for bundling the Rust toolchain on a networked machine and transferring it.

---

## Required tools per path

| Path | Windows | Linux |
|---|---|---|
| Pre-built binary | PowerShell 5+ (built into Windows 10/11) | `bash` + `tar` (present on every RHEL/Ubuntu install) |
| Source build | `cargo` (Rust toolchain) | `cargo` (Rust toolchain) |

---

## Transferring to an air-gapped machine

### Small transfer — binary only (~5 MB)

If a pre-built binary is already in `dist/`, zip just the essentials:

**Windows (PowerShell):**
```powershell
Compress-Archive -Path run.sh, install.sh, dist -DestinationPath oxide-sloc-deploy.zip
```

**Linux:**
```bash
tar -czf oxide-sloc-deploy.tar.gz run.sh install.sh dist/
```

Extract on the target machine and run `bash install.sh`.

### Full transfer — repo bundle (~500 MB, includes vendored sources for source builds)

**Linux:**
```bash
make bundle
# Produces: oxide-sloc-bundle.tar.gz
```

**Windows (PowerShell):**
```powershell
Compress-Archive -Path . -DestinationPath oxide-sloc-bundle.zip `
    -Exclude @("target", ".git", "out", "vendor")
```

Transfer the archive to the target machine, extract it, and run `bash install.sh`.

> **Why ~160 MB?** `vendor.tar.xz` (22 MB) contains all 328 Rust crate sources compressed at xz-extreme ratio from 362 MB. The `target/` compiled artifacts (4+ GB) are excluded — platform-specific and rebuilt locally. `install.sh` decompresses `vendor.tar.xz` to `vendor/` automatically before building.

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

Then run `bash install.sh` — it decompresses `vendor.tar.xz` and builds from `vendor/` automatically.

---

## CI/CD on air-gapped infrastructure

### Jenkins

The included `Jenkinsfile` auto-installs Rust on the agent if not present. For a fully offline agent, pre-install the toolchain using the steps above, then point the job at this repository. The vendored sources mean no outbound network traffic during `cargo build`.

### GitLab CI

The included `.gitlab-ci.yml` works the same way. Use a self-hosted GitLab runner with Rust pre-installed.

### GitHub Actions (internal/self-hosted runner)

Use the standard `ci.yml` workflow on a self-hosted runner. Cache `~/.cargo` and `~/.rustup` between runs to avoid re-downloading.

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
