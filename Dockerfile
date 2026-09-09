FROM rust:latest AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/news_platform_rust /app/
COPY --from=builder /app/templates /app/templates
COPY --from=builder /app/uploads /app/uploads
EXPOSE 3000
CMD ["./news_platform_rust"]