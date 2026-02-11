# Contributing

Thanks for your interest in contributing to Evolve.

## Development Setup

1. Install Rust `1.86.0` (or newer).
2. Install `just`.
3. Clone the repository and run:

```bash
just --list
just build
```

## Required Checks Before Opening a PR

Run these commands locally:

```bash
just fmt-check
just lint
just check
just test
```

For docs changes under `docs/`, also run:

```bash
cd docs
bun install
bun run build
```

## Pull Request Guidelines

- Keep PRs focused and small.
- Include tests for behavior changes.
- Update docs when public behavior changes.
- Do not merge failing CI.
- Use clear commit messages and PR descriptions.

## Code Style

- Rust: `cargo fmt` and `cargo clippy` must pass with no warnings.
- Prefer explicit, deterministic behavior over convenience.
- Avoid introducing breaking changes without documenting migration impact.

## Reporting Bugs

Open a GitHub issue with:

- Reproduction steps
- Expected behavior
- Actual behavior
- Environment details (OS, Rust version, command output)

## Community Standards

By participating, you agree to follow the project Code of Conduct.
