#!/bin/bash
# Generate seed corpus from known valid transactions

set -e

CORPUS_DIR="corpus/fuzz_decode"
mkdir -p "$CORPUS_DIR"

# Legacy transaction test vector
echo -n "f86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83" | xxd -r -p > "$CORPUS_DIR/legacy_tx_1"

# Minimal legacy tx (empty input, small values)
echo -n "f85f80018252089400000000000000000000000000000000000000008080820001a0000000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000000" | xxd -r -p > "$CORPUS_DIR/legacy_minimal" 2>/dev/null || true

# EIP-1559 transaction prefix
echo -n "02f8" > "$CORPUS_DIR/eip1559_prefix"

# Just RLP list prefixes
echo -n "c0" | xxd -r -p > "$CORPUS_DIR/rlp_empty_list"
echo -n "f8" | xxd -r -p > "$CORPUS_DIR/rlp_long_list_prefix"

echo "Seed corpus created in $CORPUS_DIR"
