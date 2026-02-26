# Evolve

Evolve is a modular Rust SDK for building blockchain applications with Ethereum compatibility.

> [!WARNING]
> **Alpha software**: Evolve is in active development and is **not production-ready**.

## What Evolve Provides

- Account-centric execution model
- Deterministic state transition engine
- Ethereum transaction and JSON-RPC compatibility
- Composable SDK modules and standards

## Getting Started

### Prerequisites

- Rust 1.86.0 or later
- [just](https://github.com/casey/just)

### Build and Validate

```bash
git clone https://github.com/01builders/evolve.git
cd evolve
just --list
just build
just test
just quality
```

### Run a Dev Node

```bash
just node-init
just node-run
```

Default RPC endpoint: `http://localhost:8545`.

### Run `evd` (and optionally `ev-node`) with Docker Compose

Run `evd` only:

```bash
docker compose up --build evd
```

Run `evd` + `ev-node`:

```bash
docker compose -f docker-compose.yml -f docker-compose.ev-node.yml up --build
```

Override image if needed:

```bash
EV_NODE_IMAGE=<your-ev-node-image> \
docker compose -f docker-compose.yml -f docker-compose.ev-node.yml up --build
```

Notes:
- `evd` JSON-RPC is exposed on `http://localhost:8545`
- `evd` gRPC is exposed on `localhost:50051`
- `ev-node` gets `EVD_GRPC_ENDPOINT=evd:50051` in the compose network
- if your `ev-node` image needs explicit startup flags, override `command` for the `ev-node` service with an extra compose override file

## Documentation

Read the docs for implementation details instead of this README.

### Start Here

- [Documentation Home](docs/pages/index.mdx)
- [SDK Overview](crates/app/sdk/README.md)
- [Module System Overview](docs/module-system/README.md)

### Build Modules and Accounts

- [Creating Modules](docs/module-system/creating-modules.md)
- [Module Architecture](docs/module-system/architecture.md)
- [Macros Reference](docs/pages/reference/macros.mdx)
- [Accounts Concept](docs/pages/concepts/accounts.mdx)

### State Collections and Storage

- [Storage in Modules](docs/module-system/storage.md)
- [Collections Reference](docs/pages/reference/collections.mdx)
- [Collections Crate](crates/app/sdk/collections/README.md)

### Pre-built Modules and Standards

- [Pre-built Modules Reference](docs/pages/reference/prebuilt.mdx)
- [Standards Reference](docs/pages/reference/standards.mdx)
- [SDK Extensions](crates/app/sdk/extensions/README.md)

### Testing, Determinism, and Operations

- [Module Testing](docs/module-system/testing.md)
- [Determinism](docs/module-system/determinism.md)
- [STF Architecture](docs/STF_ARCHITECTURE.md)
- [Server Operations](docs/guides/server.md)
- [Production Notes](docs/pages/operating/production.mdx)

## License

Apache-2.0
