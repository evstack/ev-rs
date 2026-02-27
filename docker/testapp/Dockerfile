FROM rust:bookworm AS chef

WORKDIR /app

RUN cargo install cargo-chef

FROM chef AS planner

COPY . .

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --recipe-path recipe.json --release

COPY . .

RUN cargo build -p evolve_testapp --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system evolve \
    && useradd --system --gid evolve --create-home --home-dir /home/evolve evolve \
    && mkdir -p /var/lib/evolve/data \
    && chown -R evolve:evolve /var/lib/evolve /home/evolve

WORKDIR /app

COPY --from=builder /app/target/release/testapp /usr/local/bin/testapp

RUN chown evolve:evolve /usr/local/bin/testapp /app

USER evolve:evolve

EXPOSE 8545 50051

ENTRYPOINT ["/usr/local/bin/testapp"]
