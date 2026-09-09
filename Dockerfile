# Build Stage
FROM rust:latest AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime Stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl-dev \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/news_platform_rust /app/
COPY --from=builder /app/templates /app/templates
COPY --from=builder /app/init_db.sql /app/

EXPOSE 3000
CMD ["./news_platform_rust"]