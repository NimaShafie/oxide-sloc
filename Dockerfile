# Stage 1: build the release binary
FROM rust:1.78-slim AS builder

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

EXPOSE 3000

ENTRYPOINT ["oxidesloc"]
CMD ["serve"]
