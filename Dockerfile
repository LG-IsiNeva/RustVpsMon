# --- Stage 1: build ---
FROM rust:1-slim-bookworm AS builder
WORKDIR /app

COPY . .
RUN cargo build --release

# --- Stage 2: runtime ---
FROM debian:bookworm-slim
WORKDIR /app

# ca-certificates needed for outbound TLS: SMTP (lettre) and webhooks (reqwest)
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rustmon /app/rustmon

EXPOSE 3000
CMD ["./rustmon"]
