# Stage 1: build the release binary
# Pin builder to digest so the toolchain cannot change silently under CI.
# To refresh: docker pull rust:slim && docker inspect --format '{{index .RepoDigests 0}}' rust:slim
FROM rust:slim@sha256:81099830a1e1d244607b9a7a30f3ff6ecadc52134a933b4635faba24f52840c9 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
# Verify vendor archive integrity before extracting (FIND-017).
RUN if [ -f vendor.tar.xz ] && [ ! -d vendor ]; then \
      sha256sum -c vendor.tar.xz.sha256 && tar -xJf vendor.tar.xz; \
    fi
# --offline prevents any crates.io access; the vendor/ directory (via
# .cargo/config.toml) satisfies all dependencies without the network.
RUN cargo build --release --offline -p oxide-sloc

# Stage 2: minimal runtime image
# Pin to a specific digest to prevent silent base-image substitution (FIND-006).
# To update: docker pull debian:bookworm-slim && docker inspect --format '{{index .RepoDigests 0}}' debian:bookworm-slim
FROM debian:bookworm-slim@sha256:f9c6a2fd2ddbc23e336b6257a5245e31f996953ef06cd13a59fa0a1df2d5c252

# Install Chromium for PDF export (headless).
# For a fully air-gapped Docker host, build this layer from a pre-populated
# apt mirror or use a pre-built image that already contains chromium.
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary and static assets together so OXIDE_SLOC_ROOT=/app is valid.
# The images/ directory must live alongside the binary for the web UI to serve
# icons and logos; without it every /images/... request returns 404.
COPY --from=builder /app/target/release/oxide-sloc /usr/local/bin/oxide-sloc
COPY --from=builder /app/images ./images

# Create a non-root service account and ensure the output directory is writable by it.
RUN groupadd -r sloc && useradd -r -g sloc -u 1001 sloc \
    && mkdir -p /app/out \
    && chown -R sloc:sloc /app/out

# OXIDE_SLOC_ROOT tells the server where to find images/ and other assets,
# overriding the runtime binary-location heuristic for container deployments.
ENV OXIDE_SLOC_ROOT=/app

# Point oxide-sloc at the system Chromium
ENV SLOC_BROWSER=/usr/bin/chromium
# Enable --no-sandbox for Chromium inside Docker (FIND-024).
# Chrome's kernel-namespace sandbox is unavailable in most container runtimes
# unless the container has SYS_ADMIN capability. Set this to 0 (or unset it)
# when running with --cap-add=SYS_ADMIN and a seccomp profile that permits
# the relevant syscalls, in which case the sandbox can be enabled for stronger
# isolation.
ENV SLOC_BROWSER_NOSANDBOX=1

EXPOSE 4317

USER 1001

# HEALTHCHECK verifies the /healthz endpoint is responsive (FIND-009).
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://localhost:4317/healthz >/dev/null 2>&1 || exit 1

ENTRYPOINT ["oxide-sloc"]
# --server: binds to 0.0.0.0, suppresses browser auto-open, disables desktop-only routes.
CMD ["serve", "--server"]
