# Evolve

Evolve is a simple and ergonomic Rust software development kit for building blockchain applications.

The goal of the project is to build a diverse ecosystem of users allowing applications to freely customize and innovate
without needing to start from scratch. We opted to develop a set of simple primitives allowing anyone to build a set of
modules, integrate with any VM and work with any consensus engine.

## Modules

Here is a list of modules that are supported out of the box with Evolve:

- **token** - Fungible token with mint/burn/transfer
- **gas** - Gas metering service
- **events** - Event emission system
- **escrow** - Asset escrow functionality
- **block_info** - Block header and metadata access
- **scheduler** - Transaction scheduling
- **poa** - Proof of Authority validator management
- **unique** - Non-fungible asset management
- **ns** - Name service for account naming
- **share** - Share/LP token management

## Tooling

Evolve uses standard Rust tooling:

- `cargo build` - Build the project
- `cargo test` - Run tests
- `cargo fmt` - Format code
- `cargo clippy` - Run lints