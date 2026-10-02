# ---------- build stage ----------
FROM rust:1-slim-bookworm AS builder
RUN apt-get update && \
    apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .
RUN cargo build --release

# ---------- runtime stage ----------
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends libssl3 ca-certificates && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd -r appuser && useradd -r -g appuser appuser

WORKDIR /app
COPY --from=builder /usr/src/app/target/release/wavforge /usr/local/bin/wavforge
COPY --from=builder /usr/src/app/static ./static

RUN mkdir -p /var/lib/app && chown -R appuser:appuser /var/lib/app
USER appuser

ENV BIND_ADDR=0.0.0.0:3000 \
    STORAGE_DIR=/var/lib/app \
    RUST_LOG=info
EXPOSE 3000

CMD ["/usr/local/bin/wavforge"]
