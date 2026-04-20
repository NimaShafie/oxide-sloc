# Stage 1: build the release binary
FROM rust:1.87-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libgtk-3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN cargo build --release -p oxidesloc

# Stage 2: minimal runtime image
FROM debian:bookworm-slim

# Install Chromium for PDF export (headless)
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/oxidesloc /usr/local/bin/oxidesloc

# Point oxide-sloc at the system Chromium
ENV SLOC_BROWSER=/usr/bin/chromium

EXPOSE 4317

ENTRYPOINT ["oxidesloc"]
CMD ["serve"]
