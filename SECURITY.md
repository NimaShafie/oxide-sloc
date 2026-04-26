# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| Latest release | ✅ |
| Older releases | ❌ (upgrade to latest) |

## Reporting a vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Email the maintainer directly at **nimzshafie@gmail.com** with:

- A description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fix (optional)

You will receive a response within 5 business days. Once the issue is confirmed and a fix is prepared, a coordinated disclosure will be made with credit to the reporter (unless you prefer to remain anonymous).

## Scope

oxide-sloc is a local analysis tool. The web UI (`oxide-sloc serve`) is intended for localhost use only by default. When deployed with `--server` (binding to 0.0.0.0), treat it as any other web service and apply appropriate network-level access controls.

Known areas of potential concern:

- **File system access:** The analyzer reads files in the configured scan roots. The `allowed_scan_roots` config option (currently scaffolded) is intended to restrict this in server deployments.
- **PDF generation:** Spawns a headless Chromium-based browser. The browser binary path can be controlled via the `SLOC_BROWSER` environment variable.
- **SMTP / webhook delivery:** Credentials are passed via CLI flags or environment variables (`SLOC_SMTP_PASS`, `SLOC_WEBHOOK_TOKEN`). Prefer environment variables over flags to avoid credential exposure in shell history.

## Dependency security

Dependencies are pinned via `Cargo.lock` and vendored in `vendor.tar.xz` for reproducible offline builds. Run `cargo audit` against the lock file to check for known advisories.
