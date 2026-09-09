FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/news_platform_rust /app/
COPY --from=builder /app/templates /app/templates
EXPOSE 3000
CMD ["./news_platform_rust"]