# Stage 1: build the release binary
# Pin builder to digest so the toolchain cannot change silently under CI.
# To refresh: docker pull rust:slim && docker inspect --format '{{index .RepoDigests 0}}' rust:slim
FROM rust@sha256:715efd1ccdc4a63bd6a6e2f54387fff73f904b70e610d41b4d9d74ff38e13ad3 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    xz-utils \
    libwayland-dev \
    libgtk-3-dev \
    libxdo-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
# .cargo/config.toml is gitignored (written by local airgap scripts / Jenkinsfile),
# so it is never present in the build context. Create it inline to redirect cargo
# to the vendored sources extracted from vendor.tar.xz below.
RUN mkdir -p .cargo \
    && echo '[source.crates-io]' > .cargo/config.toml \
    && echo 'replace-with = "vendored-sources"' >> .cargo/config.toml \
    && echo '' >> .cargo/config.toml \
    && echo '[source.vendored-sources]' >> .cargo/config.toml \
    && echo 'directory = "vendor"' >> .cargo/config.toml
COPY crates/ crates/
COPY docs/assets/ docs/assets/
COPY vendor.tar.xz vendor.tar.xz.sha256 ./

# Verify the vendor archive integrity and extract it.
# This must happen before `cargo build` because .cargo/config.toml points cargo
# at the vendor/ directory as the sole crate source (no network access).
RUN sha256sum -c vendor.tar.xz.sha256 \
    && tar -xJf vendor.tar.xz \
    && rm vendor.tar.xz

# Pre-flight: confirm the workspace source tree is actually present in the build
# context. If crates/ is accidentally re-added to .dockerignore this produces a
# clear, actionable error instead of a cryptic Cargo manifest failure.
RUN test -d crates/sloc-config \
    || { \
         echo "ERROR: crates/sloc-config is missing from the Docker build context." >&2; \
         echo "Check .dockerignore — crates/ must not be excluded." >&2; \
         exit 1; \
       }

RUN cargo build --release -p oxide-sloc

# Stage 2: minimal runtime image
# Pin to a specific digest to prevent silent base-image substitution.
# To update: docker pull debian:bookworm-slim && docker inspect --format '{{index .RepoDigests 0}}' debian:bookworm-slim
FROM debian@sha256:f9c6a2fd2ddbc23e336b6257a5245e31f996953ef06cd13a59fa0a1df2d5c252

# Install Chromium for PDF export (headless).
# For a fully air-gapped Docker host, build this layer from a pre-populated
# apt mirror or use a pre-built image that already contains chromium.
RUN apt-get update \
    && apt-get upgrade -y --no-install-recommends \
    && apt-get install -y --no-install-recommends \
    chromium \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary and static assets. OXIDE_SLOC_ROOT=/app tells the web server to
# look for docs/assets/ here — the image handler serves /images/:folder/:file
# from OXIDE_SLOC_ROOT/docs/assets/:folder/:file.
COPY --from=builder /app/target/release/oxide-sloc /usr/local/bin/oxide-sloc
COPY --from=builder /app/docs/assets ./docs/assets

# Create a non-root service account and ensure the output directory is writable by it.
RUN groupadd -r sloc && useradd -r -g sloc -u 1001 sloc \
    && mkdir -p /app/out \
    && chown -R sloc:sloc /app/out

# OXIDE_SLOC_ROOT tells the server where to find docs/assets/ and other assets,
# overriding the runtime binary-location heuristic for container deployments.
ENV OXIDE_SLOC_ROOT=/app

# Point oxide-sloc at the system Chromium
ENV SLOC_BROWSER=/usr/bin/chromium
# SLOC_BROWSER_NOSANDBOX is intentionally NOT set here.
# Pass -e SLOC_BROWSER_NOSANDBOX=1 at runtime when running in a container
# runtime that does not grant SYS_ADMIN (most runtimes, and required when
# cap_drop: ALL is set). With SYS_ADMIN and a permissive seccomp profile the
# sandbox can be enabled by leaving this unset.

EXPOSE 4317

USER 1001

# HEALTHCHECK verifies the /healthz endpoint is responsive.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD oxide-sloc healthz 2>/dev/null || exit 1

ENTRYPOINT ["oxide-sloc"]
# --server: binds to 0.0.0.0, suppresses browser auto-open, disables desktop-only routes.
CMD ["serve", "--server"]
