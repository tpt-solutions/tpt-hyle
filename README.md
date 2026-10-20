# tpt-hyle

**A sync-native storage primitive for offline-first apps.** By [TPT Solutions](https://github.com/tpt-solutions).

`tpt-hyle` is a pure-Rust, append-only key-value/document store whose write log *is* the replication feed.
`tpt-rift` is a CRDT sync engine that reads that feed directly. No SQLite, no shadow tables, no triggers.

Every store is encrypted at rest in a single file, and every event is signed by the node that wrote it.

> Status: **Pre-alpha.** The storage engine, encryption at rest, signed events, sync over WebSocket and WebRTC, LAN discovery, the reference server, and the wasm and UniFFI bindings are built and tested. The UniFFI Kotlin and Swift bindings are generated but not compiled, and the Docker image is written but not built. See [TODO.md](TODO.md) for what is done and what is not, and [spec.txt](spec.txt) (RFC 003) for the design.

## Crates

| Crate | Role |
|---|---|
| `tpt-hyle` | Core store: one encrypted, signed, append-only file; `no_std` core |
| `tpt-hyle-rift` | Unified API: `open()` / `connect()` |
| `tpt-hyle-cli` | Inspect, dump, verify and compact a store |
| `tpt-rift` | CRDT sync engine, `Transport` trait |
| `tpt-rift-server` | Self-hostable reference sync server |
| `tpt-rift-transport-ws` | WebSocket transport |
| `tpt-rift-transport-webrtc` | WebRTC peer-to-peer transport, signaled through the hub |
| `tpt-rift-discovery` | LAN peer discovery by UDP announcements |
| `tpt-rift-wasm` | WebAssembly bindings (in-memory replica; persisted to IndexedDB by the page) |
| `tpt-rift-uniffi` | UniFFI bindings for Kotlin and Swift, over a file-backed replica |
| `tpt-hyle-test-utils` | In-memory store, mock transports, proptest strategies |

## Quick start

Run the tests for the whole workspace:

    cargo test --workspace

Try the two basic examples:

    cargo run -p tpt-hyle --example basic
    cargo run -p tpt-hyle-rift --example two_replicas

For an end-to-end demo, run the reference server and two `notes` clients against it. The steps are in [docs/server.md](docs/server.md#demo-two-notes-clients).

## Documentation

- [docs/format.md](docs/format.md): on-disk format
- [docs/wire.md](docs/wire.md): sync wire format
- [docs/server.md](docs/server.md): running the reference server, with the notes demo
- [docs/discovery.md](docs/discovery.md): LAN discovery
- [docs/benchmarks.md](docs/benchmarks.md): benchmark method and results
- [docs/adr/](docs/adr/): architecture decision records
- [kani/README.md](kani/README.md), [fuzz/README.md](fuzz/README.md): proofs and fuzz targets
- [SECURITY.md](SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md)

Each crate has its own README and CHANGELOG.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
