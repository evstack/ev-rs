FROM rust:bookworm AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build -p evd --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/evd /usr/local/bin/evd

EXPOSE 8545 50051

ENTRYPOINT ["/usr/local/bin/evd"]
