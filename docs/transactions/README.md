# Transaction System

Evolve supports multiple transaction formats optimized for different use cases. The transaction system is organized into separate crates, allowing applications to include only what they need.

## Transaction Types

| Crate | Package | Use Case | Min Size |
|-------|---------|----------|----------|
| `tx/eth` | `evolve_tx_eth` | Ethereum-compatible transactions | ~180 bytes |
| `tx/micro` | `evolve_tx_micro` | High-throughput micropayments | **150 bytes** |

## Architecture

```
crates/app/tx/
├── eth/                    Ethereum transactions (EIP-2718)
│   ├── Legacy (0x00)       Pre-EIP-2718 transactions
│   └── EIP-1559 (0x02)     Fee market transactions
│
└── micro/                  Micro transactions
    └── MicroTx (0x83)      Timestamp-based, nonceless
```

## Choosing a Transaction Type

### Use Ethereum Transactions (`evolve_tx_eth`) when:

- You need wallet compatibility (MetaMask, etc.)
- You're building EVM-compatible applications
- You need access lists or complex gas pricing
- Sequential transaction ordering is required

### Use Micro Transactions (`evolve_tx_micro`) when:

- Throughput is critical (thousands of txs/second)
- Transactions are simple transfers
- Parallel submission from same account is needed
- Every byte matters (IoT, embedded systems)
- You don't need Ethereum wallet compatibility

## Quick Comparison

| Feature | ETH TX | Micro TX |
|---------|--------|----------|
| **Encoding** | RLP | Fixed-layout binary |
| **Signature** | secp256k1 ECDSA | Ed25519 |
| **Nonce** | Sequential (0, 1, 2...) | None (timestamp) |
| **Amount type** | U256 (32 bytes) | u128 (16 bytes) |
| **Gas fields** | Yes (price, limit, tip) | No |
| **Replay protection** | Chain ID + Nonce | Chain ID + Timestamp |
| **Parallel submit** | No (nonce conflicts) | Yes |
| **Dependencies** | alloy-*, evolve_core | alloy-primitives, ed25519-dalek |

## Documentation

- [Ethereum Transactions](./ethereum.md) - EIP-2718 compatible transactions
- [Micro Transactions](./micro.md) - High-throughput nonceless transactions
- [Replay Protection](./replay-protection.md) - How replay attacks are prevented
