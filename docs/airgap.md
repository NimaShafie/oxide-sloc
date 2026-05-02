# Air-Gap / Offline Deployment

## Choosing a transfer path

| Path | Transfer size | Prereqs on air-gapped machine | When to use |
|---|---|---|---|
| **[Airgap kit](#option-a--fully-self-contained-kit-recommended)** | ~700 MB | `bash`, `tar` (xz), `sha256sum` | Zero Rust, zero C compiler, zero internet |
| **[Pre-built binary](#option-b--pre-built-binary)** | ~5 MB | `bash`, `tar` / PowerShell | Rust and the build step not needed at all |
| **[Vendor-only](#option-c--vendor-only-source-build)** | ~30 MB + source | Rust toolchain already installed | Rust is already on the machine |
| **[scripts/install.sh](#using-scriptsinstallsh)** | varies | depends on path taken | Generic install wrapper covering all paths |

---

## Option A — Fully self-contained kit (recommended)

Run `scripts/make-airgap-kit.sh` on any **networked** machine to produce a single archive
that contains everything needed to build oxide-sloc on a machine with no internet access,
no pre-installed Rust, and no system C compiler.

### What the kit bundles

| Component | Source | Purpose |
|---|---|---|
| `rust-{ver}-{target}.tar.gz` | static.rust-lang.org | Rust host toolchain (rustc, cargo, std) |
| `rust-std-{ver}-{musl-target}.tar.gz` | static.rust-lang.org | Rust musl target standard library |
| `{arch}-linux-musl-native.tgz` | musl.cc | musl-gcc + headers + libc (no system compiler needed) |
| `vendor.tar.xz` | generated locally | All ~328 Rust crate sources (no crates.io needed) |
| `oxide-sloc-src.tar.gz` | `git archive HEAD` | Full source tree |
| `install.sh` | embedded by the script | Wires everything together and builds |

The result is a **fully static binary** — copy it anywhere on Linux and run it with no
runtime library dependencies.

### Generate the kit (networked machine)

```bash
# Auto-detect arch (Linux x86_64 or arm64):
bash scripts/make-airgap-kit.sh

# Or specify explicitly:
bash scripts/make-airgap-kit.sh linux-x86_64
bash scripts/make-airgap-kit.sh linux-arm64

# Output: oxide-sloc-airgap-kit-{platform}-v{version}.tar.gz  (~700 MB)
```

Requirements for running the script: `bash`, `curl`, `tar`, `xz-utils`, `sha256sum`, `git`,
`cargo` (just for generating `vendor.tar.xz`).

### Build on the air-gapped machine

```bash
# Transfer the kit archive via USB, SCP to an intermediate host, internal file server, etc.

tar xzf oxide-sloc-airgap-kit-linux-x86_64-v*.tar.gz
cd oxide-sloc-airgap-kit-*/
bash install.sh
```

The embedded `install.sh`:
1. Installs the Rust toolchain into `.tools/rust/` — no root, no system PATH changes.
2. Installs the musl C toolchain into `.tools/musl/` — no root.
3. Verifies and extracts the vendor crate sources.
4. Builds a fully static oxide-sloc binary in the kit directory.

After a successful build:

```bash
./oxide-sloc serve              # web UI at http://127.0.0.1:4317
./oxide-sloc analyze /path/to/repo --plain
```

### Options

```bash
bash install.sh --gnu   # use system gcc instead of bundled musl-gcc
                        # produces a dynamically linked binary (requires glibc at runtime)
```

### System requirements on the air-gapped machine

| Requirement | Notes |
|---|---|
| OS | Linux x86_64 or arm64 |
| Tools | `bash`, `tar` (with xz/`-J` flag), `sha256sum` |
| Root / sudo | **Not required** — installs to `.tools/` inside the kit directory |
| Internet | **Not required** |
| Rust | **Not required** — bundled by the kit |
| C compiler | **Not required** — bundled musl-gcc is used |

---

## Option B — Pre-built binary

Download the pre-built binary for your platform from the
[GitHub releases page](https://github.com/oxide-sloc/oxide-sloc/releases)
and transfer it alongside the `scripts/` directory.

**Linux:**
```bash
# On the networked machine — download the binary, then:
tar -czf oxide-sloc-deploy.tar.gz oxide-sloc-linux-x86_64 scripts/
# Transfer to the air-gapped machine and extract, then:
bash scripts/install.sh   # detects the binary and skips the build step
```

**Windows (PowerShell, via Git Bash):**
```powershell
Compress-Archive -Path oxide-sloc-windows-x86_64.exe, scripts `
    -DestinationPath oxide-sloc-deploy.zip
```
Extract on the target machine and run `bash scripts/install.sh` in Git Bash.

---

## Option C — Vendor-only source build

Use this when the Rust toolchain is already installed on the air-gapped machine and you
only need to transfer the crate sources.

Download `vendor.tar.xz` and `vendor.tar.xz.sha256` from the
[GitHub releases page](https://github.com/oxide-sloc/oxide-sloc/releases),
then bundle with the source tree:

```bash
# On the networked machine (after placing vendor.tar.xz in the repo root):
tar -czf oxide-sloc-bundle.tar.gz \
    --exclude=target --exclude=.git \
    vendor.tar.xz vendor.tar.xz.sha256 scripts/ Cargo.toml Cargo.lock \
    crates/ docs/ examples/ tests/ deploy/ ci/

# Transfer, then on the air-gapped machine:
tar xzf oxide-sloc-bundle.tar.gz
bash scripts/airgap-build.sh vendor.tar.xz
# Binary lands at: target/release/oxide-sloc
```

`scripts/airgap-build.sh` verifies the vendor checksum, extracts, writes
`.cargo/config.toml`, and runs `cargo build --release --offline`.

> **What vendor.tar.xz covers:** All Rust crate source dependencies (~328 crates).
> It does **not** include the Rust toolchain. If you also need to transfer the
> toolchain, use [Option A](#option-a--fully-self-contained-kit-recommended) instead.

---

## Using `scripts/install.sh`

The install script tries each path in order and stops at the first success:

1. **Binary already present** — `oxide-sloc` is in the repo root → nothing to do.
2. **Pre-built binary in `dist/`** — extracts `dist/oxide-sloc-linux-x86_64.tar.gz` (Linux) or `dist/oxide-sloc-windows-x64.zip` (Windows). No extra tools needed.
3. **Rust installed + vendor archive present** — decompresses `vendor.tar.xz` to `vendor/`, writes `.cargo/config.toml`, runs `cargo build --release --offline`.
4. **Nothing works** — prints instructions.

```bash
bash scripts/install.sh   # then:
bash scripts/run.sh       # launches the web UI
```

---

## CI/CD on air-gapped infrastructure

### Jenkins

The included `Jenkinsfile` auto-installs Rust on the agent if not present. For a fully
offline agent, use the airgap kit to install Rust and build the binary once, then copy
the binary to the agent. Or pre-install the toolchain manually and place `vendor.tar.xz`
in the workspace — the pipeline will extract it before building.

### GitLab CI

The included `.gitlab-ci.yml` works the same way. Use a self-hosted GitLab runner with
Rust pre-installed and `vendor.tar.xz` available in the runner's workspace cache or via
a custom pre-clone step.

### GitHub Actions (self-hosted runner)

For internet-connected runners, cargo downloads crates normally via Swatinem/rust-cache.
For air-gapped runners, place `vendor.tar.xz` in the workspace and add a vendor
extraction step before `cargo build`.

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

PDF export requires a locally installed Chromium-based browser (Chrome, Edge, Brave,
Vivaldi, or Opera). Set `SLOC_BROWSER=/path/to/chromium` if auto-discovery fails.
