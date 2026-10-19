# Contributing to tpt-hyle

tpt-hyle takes contributions through **issues only**. Please do not open pull
requests. Describe the bug, the missing feature, or the design question in an
issue, and the maintainers will make the change.

Good issues say what you tried, what you expected, and what happened. For a
bug, include a minimal reproduction. For a design question, say which part of
[spec.txt](spec.txt) or [TODO.md](TODO.md) it touches.

Before you open an issue, check [TODO.md](TODO.md). The item may already be
planned.

## Reporting a bug in the store

If a bug can corrupt data or make `open` panic on bad bytes, follow
[SECURITY.md](SECURITY.md) instead of opening a public issue.

## Development checks

Maintainers run these before every commit, and CI runs the same checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p tpt-hyle --no-default-features --target thumbv7em-none-eabihf
```

The MSRV is **1.75**. The core crate must stay `no_std` on its default path.

## Design decisions

The on-disk format is a compatibility promise. A change to how bytes are
written goes with an update to [docs/format.md](docs/format.md) and a bump of
`FORMAT_VERSION`. Significant design choices are recorded as numbered ADRs in
[docs/adr/](docs/adr/).

## License

The project is dual-licensed under MIT OR Apache-2.0, as described in the
[README](README.md).
