# Evolve SDK development commands
# Run `just --list` to see all available commands

# Default recipe: show available commands
default:
    @just --list

# ============================================================================
# BUILD
# ============================================================================

# Build the entire workspace in debug mode
build:
    cargo build --workspace

# Build the entire workspace in release mode
release:
    cargo build --workspace --release

# Check compilation without producing binaries (faster than build)
check:
    cargo check --workspace --all-targets

# ============================================================================
# QUALITY
# ============================================================================

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all quality checks (format, lint, check)
quality: fmt-check lint check

# ============================================================================
# TESTING
# ============================================================================

# Run all tests
test:
    cargo test --workspace

# Run tests for a specific package
test-pkg pkg:
    cargo test -p {{pkg}}

# Run simulation tests only
test-sim:
    cargo test -p evolve_testapp simulation

# Run e2e tests only
test-e2e:
    cargo test -p evolve_testapp e2e

# Run tests with output shown
test-verbose:
    cargo test --workspace -- --nocapture

# Run a specific test by name
test-one name:
    cargo test --workspace {{name}} -- --nocapture

# ============================================================================
# DEV NODE
# ============================================================================

# Initialize the dev node (creates data directory and genesis)
node-init:
    cargo run -p evolve_testapp -- init

# Run the dev node (RPC at localhost:8545)
node-run:
    cargo run -p evolve_testapp -- run

# Run the dev node in release mode
node-run-release:
    cargo run -p evolve_testapp --release -- run

# Clean dev node data and reinitialize
node-reset:
    rm -rf .data && just node-init

# ============================================================================
# SIMULATION CLI
# ============================================================================

# Run a simulation with default parameters (100 blocks)
sim:
    cargo run -p evolve-sim -- run

# Run a simulation with a specific seed for reproducibility
sim-seed seed:
    cargo run -p evolve-sim -- run --seed {{seed}}

# Run a simulation with custom block count
sim-blocks n:
    cargo run -p evolve-sim -- run -n {{n}}

# Run a simulation with tracing enabled
sim-trace output="trace.json":
    cargo run -p evolve-sim -- run --trace {{output}}

# Run simulation with fault injection
sim-faults read="0.01" write="0.01":
    cargo run -p evolve-sim -- run --read-fault-prob {{read}} --write-fault-prob {{write}}

# Replay a trace file
sim-replay trace:
    cargo run -p evolve-sim -- replay --trace {{trace}}

# Replay a trace in interactive mode
sim-debug trace:
    cargo run -p evolve-sim -- replay --trace {{trace}} --interactive

# Generate a report from a trace
sim-report trace:
    cargo run -p evolve-sim -- report --trace {{trace}}

# ============================================================================
# BENCHMARKS
# ============================================================================

# Run STF benchmarks
bench:
    cargo bench -p evolve_testapp

# Run benchmarks and save baseline
bench-save name:
    cargo bench -p evolve_testapp -- --save-baseline {{name}}

# Compare benchmarks against a baseline
bench-compare baseline:
    cargo bench -p evolve_testapp -- --baseline {{baseline}}

# ============================================================================
# FUZZING
# ============================================================================

# Run fuzz testing on transaction decoding (requires cargo-fuzz)
fuzz-decode:
    cd crates/app/tx/fuzz && cargo +nightly fuzz run fuzz_decode

# Run fuzz testing on transaction roundtrip
fuzz-roundtrip:
    cd crates/app/tx/fuzz && cargo +nightly fuzz run fuzz_roundtrip

# Run fuzz testing on structured transactions
fuzz-structured:
    cd crates/app/tx/fuzz && cargo +nightly fuzz run fuzz_structured

# Seed the fuzz corpus
fuzz-seed:
    cd crates/app/tx/fuzz && ./seed_corpus.sh

# ============================================================================
# DOCUMENTATION
# ============================================================================

# Build documentation
doc:
    cargo doc --workspace --no-deps

# Build and open documentation in browser
doc-open:
    cargo doc --workspace --no-deps --open

# ============================================================================
# CLEANUP
# ============================================================================

# Remove build artifacts
clean:
    cargo clean

# Remove build artifacts and dev node data
clean-all:
    cargo clean
    rm -rf .data

# ============================================================================
# PRE-COMMIT
# ============================================================================

# Quick pre-commit check (format, lint, check compilation)
pre-commit: fmt lint check
