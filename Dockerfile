# Multi-stage build for ovp-data-api
# Layer cache strategy: deps compile once, src changes recompile app only.

FROM rust:1.94-bookworm AS builder
WORKDIR /app

# Prime the dep cache with a stub main.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release \
 && rm -rf src target/release/deps/ovp_data_api*

COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ovp-data-api /usr/local/bin/ovp-data-api
COPY --from=builder /app/migrations /migrations

ENV RUST_LOG=info
EXPOSE 8001
CMD ["ovp-data-api"]
