# Stage 1: build the release binary
FROM rust:slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
# --offline prevents any crates.io access; the vendor/ directory (via
# .cargo/config.toml) satisfies all dependencies without the network.
RUN cargo build --release --offline -p oxide-sloc

# Stage 2: minimal runtime image
FROM debian:bookworm-slim

# Install Chromium for PDF export (headless).
# For a fully air-gapped Docker host, build this layer from a pre-populated
# apt mirror or use a pre-built image that already contains chromium.
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium \
    ca-certificates \
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

EXPOSE 4317

USER 1001

ENTRYPOINT ["oxide-sloc"]
# --server: binds to 0.0.0.0, suppresses browser auto-open, disables desktop-only routes.
CMD ["serve", "--server"]
