# Transaction System

Evolve supports Ethereum-compatible transactions through the `evolve_tx_eth` crate.

## Transaction Types

| Crate | Package | Use Case | Min Size |
|-------|---------|----------|----------|
| `tx/eth` | `evolve_tx_eth` | Ethereum-compatible transactions | ~180 bytes |

## Architecture

```
crates/app/tx/
├── eth/                    Ethereum transactions (EIP-2718)
│   ├── Legacy (0x00)       Pre-EIP-2718 transactions
│   └── EIP-1559 (0x02)     Fee market transactions
```

## Choosing a Transaction Type

### Use Ethereum Transactions (`evolve_tx_eth`) when:

- You need wallet compatibility (MetaMask, etc.)
- You're building EVM-compatible applications
- You need access lists or complex gas pricing
- Sequential transaction ordering is required

## Documentation

- [Ethereum Transactions](./ethereum.md) - EIP-2718 compatible transactions
- [Replay Protection](./replay-protection.md) - How replay attacks are prevented
