# Evolve SDK development commands
# Run `just --list` to see all available commands

# Default recipe: show available commands
[private]
default:
    @just --list --unsorted

# ============================================================================
# BUILD
# ============================================================================

# Build the entire workspace in debug mode
[group('build')]
build:
    cargo build --workspace

# Build the entire workspace in release mode
[group('build')]
release:
    cargo build --workspace --release

# Build testapp binary
[group('build')]
build-testapp:
    cargo build -p evolve_testapp

# Build testapp binary in release mode
[group('build')]
build-testapp-release:
    cargo build -p evolve_testapp --release

# Build evd binary
[group('build')]
build-evd:
    cargo build -p evd

# Build evd binary in release mode
[group('build')]
build-evd-release:
    cargo build -p evd --release

# Check compilation without producing binaries (faster than build)
[group('build')]
check:
    cargo check --workspace --all-targets

# ============================================================================
# QUALITY
# ============================================================================

# Format all code
[group('quality')]
fmt:
    cargo fmt --all

# Check formatting without modifying files
[group('quality')]
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints
[group('quality')]
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all quality checks (format, lint, check)
[group('quality')]
quality: fmt-check lint check

# Quick pre-commit check (format, lint, check compilation)
[group('quality')]
pre-commit: fmt lint check

# ============================================================================
# TESTING
# ============================================================================

# Run all tests
[group('test')]
test:
    cargo test --workspace

# Run tests for a specific package
[group('test')]
test-pkg pkg:
    cargo test -p {{pkg}}

# Run simulation tests only
[group('test')]
test-sim:
    cargo test -p evolve_testapp simulation

# Run e2e tests only
[group('test')]
test-e2e:
    cargo test -p evolve_testapp e2e

# Run tests with output shown
[group('test')]
test-verbose:
    cargo test --workspace -- --nocapture

# Run a specific test by name
[group('test')]
test-one name:
    cargo test --workspace {{name}} -- --nocapture

# Run long-running simulation tests (ignored by default)
[group('test')]
test-long:
    cargo test -p evolve-testapp --test simulation_long_tests -- --ignored --nocapture

# ============================================================================
# DEV NODE (testapp)
# ============================================================================

# Initialize the dev node (creates data directory and genesis)
[group('node')]
node-init:
    cargo run -p evolve_testapp -- init

# Run the dev node (RPC at localhost:8545)
[group('node')]
node-run:
    cargo run -p evolve_testapp -- run

# Run the dev node in release mode
[group('node')]
node-run-release:
    cargo run -p evolve_testapp --release -- run

# Clean dev node data and reinitialize
[group('node')]
node-reset:
    rm -rf .data && just node-init

# ============================================================================
# EVD (gRPC node)
# ============================================================================

# Initialize evd genesis (default testapp genesis; pass genesis file for custom ETH accounts)
[group('evd')]
evd-init genesis="":
    cargo run -p evd -- init {{ if genesis != "" { "--genesis-file " + genesis } else { "" } }}

# Run evd (gRPC on :50051, JSON-RPC on :8545)
[group('evd')]
evd-run genesis="":
    cargo run -p evd -- run {{ if genesis != "" { "--genesis-file " + genesis } else { "" } }}

# Run evd in release mode
[group('evd')]
evd-run-release genesis="":
    cargo run -p evd --release -- run {{ if genesis != "" { "--genesis-file " + genesis } else { "" } }}

# Run evd with custom addresses
[group('evd')]
evd-run-custom genesis grpc="0.0.0.0:50051" rpc="0.0.0.0:8545":
    cargo run -p evd -- run {{ if genesis != "" { "--genesis-file " + genesis } else { "" } }} --grpc-addr {{grpc}} --rpc-addr {{rpc}}

# Clean evd data and reinitialize
[group('evd')]
evd-reset genesis="":
    rm -rf ./data && just evd-init {{genesis}}

# ============================================================================
# SIMULATION
# ============================================================================

# Run a simulation with default parameters (100 blocks)
[group('sim')]
sim:
    cargo run -p evolve-sim -- run

# Run a simulation with a specific seed for reproducibility
[group('sim')]
sim-seed seed:
    cargo run -p evolve-sim -- run --seed {{seed}}

# Run a simulation with custom block count
[group('sim')]
sim-blocks n:
    cargo run -p evolve-sim -- run -n {{n}}

# Run a simulation with tracing enabled
[group('sim')]
sim-trace output="trace.json":
    cargo run -p evolve-sim -- run --trace {{output}}

# Run simulation with fault injection
[group('sim')]
sim-faults read="0.01" write="0.01":
    cargo run -p evolve-sim -- run --read-fault-prob {{read}} --write-fault-prob {{write}}

# Replay a trace file
[group('sim')]
sim-replay trace:
    cargo run -p evolve-sim -- replay --trace {{trace}}

# Replay a trace in interactive mode
[group('sim')]
sim-debug trace:
    cargo run -p evolve-sim -- replay --trace {{trace}} --interactive

# Generate a report from a trace
[group('sim')]
sim-report trace:
    cargo run -p evolve-sim -- report --trace {{trace}}

# ============================================================================
# BENCHMARKS
# ============================================================================

# Run STF benchmarks
[group('bench')]
bench:
    cargo bench -p evolve_testapp

# Run benchmarks and save baseline
[group('bench')]
bench-save name:
    cargo bench -p evolve_testapp -- --save-baseline {{name}}

# Compare benchmarks against a baseline
[group('bench')]
bench-compare baseline:
    cargo bench -p evolve_testapp -- --baseline {{baseline}}

# ============================================================================
# FUZZING
# ============================================================================

# Run fuzz testing on transaction decoding (requires cargo-fuzz)
[group('fuzz')]
fuzz-decode:
    cd crates/app/tx/fuzz && cargo +nightly fuzz run fuzz_decode

# Run fuzz testing on transaction roundtrip
[group('fuzz')]
fuzz-roundtrip:
    cd crates/app/tx/fuzz && cargo +nightly fuzz run fuzz_roundtrip

# Run fuzz testing on structured transactions
[group('fuzz')]
fuzz-structured:
    cd crates/app/tx/fuzz && cargo +nightly fuzz run fuzz_structured

# Seed the fuzz corpus
[group('fuzz')]
fuzz-seed:
    cd crates/app/tx/fuzz && ./seed_corpus.sh

# ============================================================================
# DOCUMENTATION
# ============================================================================

# Build documentation
[group('docs')]
doc:
    cargo doc --workspace --no-deps

# Build and open documentation in browser
[group('docs')]
doc-open:
    cargo doc --workspace --no-deps --open

# ============================================================================
# CLEANUP
# ============================================================================

# Remove build artifacts
[group('cleanup')]
clean:
    cargo clean

# Remove build artifacts and dev node data
[group('cleanup')]
clean-all:
    cargo clean
    rm -rf .data
