# Server deployment (alternative install)

> **Default install:** run `bash scripts/run.sh` (or `cargo run`) for personal local use.
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
sudo mkdir -p /opt/oxide-sloc/{docs/assets,out}
sudo chown -R oxide-sloc:oxide-sloc /opt/oxide-sloc
sudo cp -r docs/assets/ /opt/oxide-sloc/docs/assets/
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

## Security

### Authentication (`SLOC_API_KEY`)

Set `SLOC_API_KEY` to require a bearer token on every request. When set, clients must supply one of:

- `Authorization: Bearer <key>` header
- `X-API-Key: <key>` header

When running in `--server` mode without `SLOC_API_KEY`, oxide-sloc logs a warning at startup. In local mode the warning is suppressed.

### TLS (`SLOC_TLS_CERT` / `SLOC_TLS_KEY`)

oxide-sloc can terminate TLS directly using PEM-encoded certificate and key files:

```bash
SLOC_TLS_CERT=/etc/oxide-sloc/server.crt \
SLOC_TLS_KEY=/etc/oxide-sloc/server.key \
oxide-sloc serve --server
```

When both are set the server prints `OxideSLOC server running at https://… (TLS)` and accepts HTTPS connections. When running in `--server` mode without TLS, a cleartext warning is logged at startup.

For production deployments a **reverse proxy** (Nginx, Caddy) with `bind_address = "127.0.0.1:4317"` is the preferred approach — TLS termination at the proxy layer avoids certificate management in the binary.

### Rate limiting

A sliding-window rate limiter enforces **60 requests per 60-second window per client IP** across all routes. Requests over the limit receive `HTTP 429 Too Many Requests`.

---

## Environment variables

| Variable | Purpose | Default |
|---|---|---|
| `OXIDE_SLOC_ROOT` | Directory whose `docs/assets/` subtree the web server serves as `/images/*` | binary directory |
| `SLOC_BROWSER` | Path to Chromium-based browser for PDF export | auto-detected |
| `SLOC_BROWSER_NOSANDBOX` | Set to `1` to add `--no-sandbox` to Chromium args (required in Docker) | unset |
| `SLOC_REGISTRY_PATH` | Override path for `registry.json` | `<out-dir>/registry.json` |
| `SLOC_API_KEY` | Bearer token for request authentication (server mode) | unset (no auth) |
| `SLOC_TLS_CERT` | Path to PEM certificate file for native TLS | unset |
| `SLOC_TLS_KEY` | Path to PEM private key file for native TLS | unset |
| `RUST_LOG` | Tracing log level (`info`, `debug`, `warn`) | `info` |

---

## Health check

`GET /healthz` returns `200 OK` with body `ok`.

```bash
curl http://localhost:4317/healthz
```

The Docker image includes a `HEALTHCHECK` that polls this endpoint every 30 seconds.
