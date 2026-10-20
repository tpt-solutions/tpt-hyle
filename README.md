# tpt-hyle

**A sync-native storage primitive for offline-first apps.** By [TPT Solutions](https://github.com/tpt-solutions).

`tpt-hyle` is a pure-Rust, append-only key-value/document store whose write log *is* the replication feed.
`tpt-rift` is a CRDT sync engine that reads that feed directly. No SQLite, no shadow tables, no triggers.

> Status: **Pre-alpha.** The storage engine, sync, transports (WebSocket, WebRTC), the reference server, and bindings (wasm, UniFFI) are built and tested. Encryption at rest and signed events are in. The UniFFI Kotlin and Swift bindings are generated but not compiled. Docker is not built. See [TODO.md](TODO.md) and [spec.txt](spec.txt) (RFC 003).

## Crates

| Crate | Role |
|---|---|
| `tpt-hyle` | Core store: one encrypted, signed, append-only file; `no_std` core |
| `tpt-rift` | CRDT sync engine, `Transport` trait |
| `tpt-hyle-rift` | Unified API: `open()` / `connect()` |
| `tpt-rift-server` | Self-hostable reference sync server |
| `tpt-hyle-cli` | Inspect, dump, verify, print public key, compact |
| `tpt-rift-transport-ws` | WebSocket transport |
| `tpt-rift-transport-webrtc` | WebRTC peer-to-peer transport |
| `tpt-rift-wasm` | WebAssembly bindings (IndexedDB backing) |
| `tpt-hyle-test-utils` | In-memory store, model-checking ops, proptest strategies |

## Documentation

- [docs/format.md](docs/format.md): on-disk format
- [docs/adr/](docs/adr/): architecture decision records
- [SECURITY.md](SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md)

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
