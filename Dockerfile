# ---- Build stage ----
FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src/ src/
COPY frontend/ frontend/
RUN cargo build --release

# ---- Runtime stage ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/os-pulse /usr/local/bin/os-pulse
EXPOSE 3000
ENV OSP_INTERVAL=1
CMD ["os-pulse"]
