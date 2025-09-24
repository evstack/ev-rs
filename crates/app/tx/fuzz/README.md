# Transaction Fuzzing

Fuzz targets for the `evolve_tx` crate to find bugs in transaction decoding,
encoding, and verification.

## Prerequisites

Install cargo-fuzz:

```bash
cargo install cargo-fuzz
```

You'll need nightly Rust:

```bash
rustup install nightly
```

## Fuzz Targets

### fuzz_decode

Primary attack surface - feeds arbitrary bytes to `TxEnvelope::decode()`.
Catches panics, infinite loops, and memory issues in the decoder.

```bash
cargo +nightly fuzz run fuzz_decode
```

### fuzz_roundtrip

Tests encode/decode consistency. If a transaction decodes successfully,
re-encoding and decoding should produce identical results.

```bash
cargo +nightly fuzz run fuzz_roundtrip
```

### fuzz_structured

Uses the `arbitrary` crate to generate valid-looking transaction structures,
then encodes and decodes them. Tests the full pipeline including signature
recovery paths.

```bash
cargo +nightly fuzz run fuzz_structured
```

## Using the Dictionary

For better coverage, use the transaction dictionary:

```bash
cargo +nightly fuzz run fuzz_decode -- -dict=tx.dict
```

## Seed Corpus

Generate initial seed corpus:

```bash
chmod +x seed_corpus.sh
./seed_corpus.sh
```

Then run with seeds:

```bash
cargo +nightly fuzz run fuzz_decode corpus/fuzz_decode
```

## Running with Options

Run for a specific duration:

```bash
cargo +nightly fuzz run fuzz_decode -- -max_total_time=300
```

Run with multiple jobs:

```bash
cargo +nightly fuzz run fuzz_decode -- -jobs=4 -workers=4
```

Minimize a crash:

```bash
cargo +nightly fuzz tmin fuzz_decode artifacts/fuzz_decode/crash-XXX
```

## Coverage

Generate coverage report:

```bash
cargo +nightly fuzz coverage fuzz_decode
```

## What to Look For

- **Panics**: Unwrap failures, index out of bounds, integer overflow
- **Infinite loops**: Hangs during decoding
- **Memory exhaustion**: Large allocations from small inputs
- **Logic bugs**: Roundtrip mismatches, incorrect field values
