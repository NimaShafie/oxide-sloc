# Server deployment (alternative install)

> **Default install:** run `bash run.sh` (or `cargo run`) for personal local use.
> This guide is for hosting oxide-sloc persistently so **other users** can reach it.

---

## Local vs server mode

| | Local (default) | Server (`--server`) |
|---|---|---|
| Bind address default | `127.0.0.1:4317` | `0.0.0.0:4317` |
| Browser auto-open | yes | no |
| Native file picker | yes | disabled (404) |
| OS path opener | yes | disabled (404) |
| Startup message | "local web UI" | "server" |

In server mode users enter paths manually into the web form.
The scan registry and report artifacts are shared across all sessions.

---

## Option A — Docker Compose (recommended)

```bash
# Build and start (survives reboots via restart: unless-stopped)
docker compose up -d

# Tail logs
docker compose logs -f

# Stop
docker compose down
```

The container runs with `--server` by default — binds to `0.0.0.0:4317`,
browser auto-open is suppressed, and file-picker routes are disabled.

### Analyzing paths on the host

Mount any directory you want users to be able to analyze:

```yaml
volumes:
  - type: bind
    source: /path/to/project
    target: /repo
    read_only: true
```

Users then enter `/repo` in the path field of the web form.

---

## Option B — systemd (Linux bare-metal / VPS)

### 1. Install the binary

```bash
# From a release archive:
tar xzf oxide-sloc-linux-x86_64.tar.gz
sudo install -m 755 oxide-sloc /usr/local/bin/oxide-sloc

# Or build from source:
cargo build --release -p oxide-sloc
sudo install -m 755 target/release/oxide-sloc /usr/local/bin/oxide-sloc
```

### 2. Create a dedicated user and working directory

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin oxide-sloc
sudo mkdir -p /opt/oxide-sloc/{images,out}
sudo chown -R oxide-sloc:oxide-sloc /opt/oxide-sloc
sudo cp -r images/ /opt/oxide-sloc/images/
```

### 3. Optional: install a config file

```bash
sudo mkdir -p /etc/oxide-sloc
sudo cp deploy/server.toml /etc/oxide-sloc/config.toml
# Edit bind address, report title, etc. as needed.
```

To use it, change `ExecStart` in the service unit to add `--config`:

```
ExecStart=/usr/local/bin/oxide-sloc serve --server --config /etc/oxide-sloc/config.toml
```

### 4. Install and enable the service

```bash
sudo cp deploy/oxide-sloc.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now oxide-sloc
```

```bash
sudo systemctl status oxide-sloc
sudo journalctl -u oxide-sloc -f
```

---

## Reverse proxy (Nginx example)

Put Nginx in front for HTTPS or a custom domain.
Use `bind_address = "127.0.0.1:4317"` in `server.toml` so the port is not directly internet-accessible.

```nginx
server {
    listen 443 ssl;
    server_name sloc.example.com;

    # ... ssl_certificate / ssl_certificate_key ...

    location / {
        proxy_pass http://127.0.0.1:4317;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

---

## Environment variables

| Variable | Purpose | Default |
|---|---|---|
| `OXIDE_SLOC_ROOT` | Directory containing `images/` assets | binary directory |
| `SLOC_BROWSER` | Path to Chromium-based browser for PDF export | auto-detected |
| `SLOC_REGISTRY_PATH` | Override path for `registry.json` | `<out-dir>/registry.json` |
| `RUST_LOG` | Tracing log level (`info`, `debug`, `warn`) | `info` |

---

## Health check

`GET /healthz` returns `200 OK` with body `ok`.

```bash
curl http://localhost:4317/healthz
```
