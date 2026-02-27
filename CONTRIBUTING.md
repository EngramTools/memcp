# Contributing to memcp

We welcome contributions! Before submitting a PR, please sign the CLA.

[![CLA assistant](https://cla-assistant.io/readme/badge/EngramTools/memcp)](https://cla-assistant.io/EngramTools/memcp)

## Contributor License Agreement

The CLA bot will comment on your first PR. Click the link in the comment to sign. The full CLA text is in [.github/cla.md](.github/cla.md).

## Getting Started

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/your-feature`
3. Make your changes
4. Run tests: `cargo test`
5. Run clippy: `cargo clippy`
6. Commit with a clear message
7. Push and open a PR

## Development

memcp is written in Rust (2021 edition) using Tokio and rmcp.

```bash
cargo build        # Build
cargo test         # Run tests
cargo clippy       # Lint
cargo run          # Run locally
```

## License

memcp is licensed under the [Business Source License 1.1](LICENSE). By contributing, you agree to the terms of the CLA.
