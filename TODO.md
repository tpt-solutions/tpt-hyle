# tpt-hyle TODO

Source of truth for design: [spec.txt](spec.txt) (RFC 003). Tick items as they land.
On-disk format: [docs/format.md](docs/format.md). Design decisions: [docs/adr/](docs/adr/).

## Decisions made (deviations from spec)

- [x] License: `MIT OR Apache-2.0`, copyright TPT Solutions
- [x] `tpt-hyle` uses layered features: bare `no_std`/no-alloc core -> `alloc` (in-memory storage, compaction) -> `std` (file backend). JSON helpers sit behind a separate `json` feature, so `alloc` stays dependency-free
- [x] `ChangeEvent`: replace single `vector_clock: u64` with `lamport: u64` + `node_id`; conflict order = `(lamport, node_id)`. (The spec's key-hash tie-break gives both peers the same input, so it can't separate them.)
- [x] MSRV 1.75, edition 2021
- [x] Repo: `github.com/tpt-solutions/tpt-hyle`

## Phase 0: Scaffold

- [x] Cargo workspace with 9 crates (compiles)
- [x] `LICENSE-MIT`, `LICENSE-APACHE`, README, .gitignore
- [x] CI workflow (fmt, clippy, test, no_std target, MSRV)
- [ ] `git init`, create GitHub repo, push (local repo already exists; remote creation and push not done)
- [x] Add `SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`
- [x] Add `rustfmt.toml`, `clippy.toml`, `deny.toml` (cargo-deny: licenses, bans and sources pass; advisories not checked, needs network)
- [x] Set up `cargo-fuzz` / `cargo-kani` directories (`fuzz/` target is written but not built: needs nightly and libFuzzer; `kani/` is a README only)
- [x] Per-crate README + crates.io metadata
- [ ] Reserve crate names on crates.io (requires publishing, not done)
- [x] Decide `node_id` width (u128) and write ADR ([ADR 0001](docs/adr/0001-node-id-width.md))
- [x] Write on-disk format spec in `docs/format.md` (magic, version, endianness, checksums)
- [x] Finalize `ChangeEvent` layout (key length, node_id, CRC, flags; 56-byte record)

## Phase 1: tpt-hyle core (Year 1)

- [x] `Storage` trait (append/read/truncate/flush/len; no `write_at`, since the engine is append-only)
- [x] In-memory fixed-buffer backend (no_alloc): `FixedStorage<N>`
- [x] File backend (`std`): `FileStorage`, `Hyle::open_file`
- [ ] Optional mmap backend (blocked: needs `unsafe`, and the crate forbids it. Revisit if benchmarks need it)
- [x] Segment headers + magic/version + per-record CRC-32C (SHA-256 not used, see [ADR 0002](docs/adr/0002-integrity-and-indexing.md))
- [x] State Segment append (key + value records)
- [x] Sync Segment append (`ChangeEvent`), fixed-size records
- [x] Atomic dual-append with crash recovery (torn-tail truncation, orphan state removal, mid-log corruption refused)
- [x] Batched commits with commit-marker records: a batch is durable only when its commit record is. Recovery drops an uncommitted tail whole and refuses damage in committed data ([ADR 0005](docs/adr/0005-batched-commits.md))
- [x] Batch rollback: dropping an uncommitted batch truncates both segments back to where it started
- [x] `put` / `get` / `delete` (tombstones)
- [x] Key index (hash -> sync offsets, `alloc` only). Built once the benchmark showed linear reads: at 50k keys a get took 4.4 ms, now 1.0 us. Rebuilt on open, retracted on batch rollback. Without `alloc`, lookups still scan ([ADR 0008](docs/adr/0008-key-index.md))
- [x] Hash-collision handling for `key_hash: u64` (full-key compare)
- [x] Sequential Sync Segment reader from offset (`events_since`)
- [x] Lamport clock + node_id management
- [x] Compaction: rewrite State, truncate Sync, crash-safe swap (`compact_into`, `compact_file`; see [ADR 0003](docs/adr/0003-compaction-and-generations.md))
- [x] Define what compaction means for peers behind the truncation point: generations, latest-per-key semantics (ADR 0003)
- [ ] Snapshot/resync wire protocol for peers with a stale generation (Phase 2)
- [x] `alloc`-level JSON helpers (`put_json` / `get_json`), behind the `json` feature
- [x] Error type (hand-rolled, with `std::error::Error` under `std`; see ADR 0002)
- [x] `tpt-hyle-test-utils`: in-memory store, proptest generators (mock transports belong to Phase 2)
- [x] `tpt-hyle-cli`: `inspect`, `dump-sync-log`, `verify`, `compact`
- [x] Corruption tests: truncated/flipped-bit files never panic (exhaustive single-bit sweep, truncation property, fuzz target)
- [x] Property tests against a reference model, including torn-tail recovery
- [x] Stress tests: 10,000-op random histories with reopens and compactions, file-backed variant, feed replay checked against state
- [ ] Benchmarks vs SQLite: the 10x target was not reached. Current numbers and method are in [docs/benchmarks.md](docs/benchmarks.md). In memory, tpt-hyle is 0.085x SQLite, because each write is signed. Durable one-commit-per-write is 1.03x. Batches of 100 are 0.44x. Harness: `bench/` (outside the workspace)
- [x] CI builds core on `thumbv7em-none-eabihf` and `wasm32-unknown-unknown` (wasm steps added to CI)

## Phase 2: tpt-rift + unified API (Year 2)

- [x] `Transport` trait. Decided: deltas carry key and value, because offsets are meaningless to a peer ([ADR 0004](docs/adr/0004-replication-semantics.md))
- [x] Wire format for deltas (keys, values, lamport, node id, tombstone), versioned ([docs/wire.md](docs/wire.md))
- [x] Last-writer-wins resolver per key, ordered by `(lamport, node_id)`. Named "LWW register per key": there is no element set yet
- [x] Apply remote events into tpt-hyle without echoing them back (origin tracking; forwarding through a middle node still works)
- [x] Per-peer sync cursors `(generation, offset)`, advanced only after delivery
- [x] Background sync loop: `run_until` with backoff, and `run_reconnecting` with reconnection after drops, a `Trigger` for local writes, and a timer ([ADR 0009](docs/adr/0009-reconnecting-sync.md))
- [x] Basic JSON CRDT: field-level LWW, one store key per leaf ([ADR 0007](docs/adr/0007-json-document-crdt.md)). Arrays are whole-value leaves; element-level merge is not built
- [x] Mock transport and two-instance offline-write convergence test
- [x] `tpt-hyle-rift`: `open()`, `connect()`, `resume()` (with automatic recovery from a stale generation).
- [x] Property test: convergence under reordering and duplication (mutation-checked)
- [x] Examples in `crates/tpt-hyle-rift/examples/` (`two_replicas`)

## Phase 3: Client-server (Year 3)

- [x] `tpt-rift-transport-ws` client + server halves (no auth or TLS yet; see the Phase 3 auth item)
- [x] `tpt-rift-server`: WebSocket hub with fan-out, per-connection cursors, and size limits ([docs/server.md](docs/server.md))
- [x] Authentication: bearer tokens stored as SHA-256 hashes, each bound to one node id. JWT and mTLS deferred ([ADR 0006](docs/adr/0006-server-auth-and-access.md))
- [x] Access control: per-token key-prefix read and write lists
- [x] Multi-client fan-out (polling hub; each connection keeps its own cursor)
- [x] Message size limit and per-connection rate limit (token bucket, disconnect on excess)
- [ ] TLS: deliberately not in the server. Expected from a reverse proxy, documented in docs/server.md
- [ ] Docker image and deploy docs: Dockerfile and docs written, not built (no Docker daemon was running here)
- [x] End-to-end demo app syncing to self-hosted server (`notes` example, walkthrough in [docs/server.md](docs/server.md#demo-two-notes-clients); run with two clients against a local hub)

## Phase 4: P2P + edge (Year 4)

- [x] `tpt-rift-transport-webrtc`: data-channel transport, signaling relayed through the hub ([ADR 0012](docs/adr/0012-webrtc-signaling.md)). STUN is tested; TURN is passed through but untested
- [x] LAN discovery by UDP broadcast, with unauthenticated announcements ([ADR 0011](docs/adr/0011-lan-discovery.md)). The broadcast path itself is not tested; loopback tests use unicast
- [x] `tpt-rift-wasm`: wasm-bindgen API over an in-memory store, with IndexedDB persistence in JavaScript (`web/persistence.js`). Tested under Node, not in a browser ([ADR 0010](docs/adr/0010-wasm-bindings.md))
- [ ] Browser demo: offline web app syncing P2P (the Node demo covers the same calls; no page yet)
- [ ] Kani harnesses: 6 verified on the format v4 layout (header, frame header, data and commit decoding, fixed-storage reads and truncation). Run in WSL, see [kani/README.md](kani/README.md). Not yet: allocator bounds, and invariants through compaction and recovery
- [x] CRDT determinism tests: convergence under random order and duplication for LWW and JSON documents (mutation-checked). Not a machine-checked proof

## Phase 5: Mobile + ecosystem (Year 5)

- [ ] 100% Kani coverage of core parser; publish proofs (5 harnesses so far, on the parsers that take untrusted bytes)
- [x] Swift (iOS) and Kotlin (Android) SDKs via UniFFI: `tpt-rift-uniffi`, Rust surface tested, bindings generated. Not compiled for either platform (no Xcode, no Android SDK here)

- [ ] Nested JSON CRDT (concurrent edits in same object)
- [ ] Integrations: tpt-aion, tpt-archon, tpt-mnemosyne
- [ ] Docs site, benchmarks page, 1.0 stability policy

## Open questions

- [x] Security model: encryption at rest (format v3, [ADR 0013](docs/adr/0013-encryption-and-single-file.md)) and signed events (format v4, [ADR 0015](docs/adr/0015-signed-events.md)). Still open: a passphrase holder can sign as its node, and a client can sign a false ack about its own state, which can defeat tombstone collection. Acks are signed, so no other node can forge one. Decided: keep per-event signing on every store (see [ADR 0015](docs/adr/0015-signed-events.md), decision on write cost) ([ADR 0014](docs/adr/0014-tombstone-collection.md))
- [ ] Schema/versioning and migration story
- [ ] Is "100% formal verification" realistic with serde in the path? Scope proofs to the no_alloc core.
- [ ] Spec's `LWW-ES` vs JSON CRDT selection: per-key type tag or per-namespace?
- [x] Deletion/tombstone GC across peers. Clients ack the hub's log position and clock; `tpt-rift-server collect` drops tombstones every configured client has acked. A client that never connects blocks collection. See [ADR 0014](docs/adr/0014-tombstone-collection.md)
