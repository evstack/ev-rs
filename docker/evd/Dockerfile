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
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system evolve \
    && useradd --system --gid evolve --create-home --home-dir /home/evolve evolve \
    && mkdir -p /var/lib/evolve/data \
    && chown -R evolve:evolve /var/lib/evolve /home/evolve

WORKDIR /app

COPY --from=builder /app/target/release/evd /usr/local/bin/evd

RUN chown evolve:evolve /usr/local/bin/evd /app

USER evolve:evolve

EXPOSE 8545 50051

ENTRYPOINT ["/usr/local/bin/evd"]
