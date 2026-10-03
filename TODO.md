# tpt-hyle TODO

Source of truth for design: [spec.txt](spec.txt) (RFC 003). Tick items as they land.

## Decisions made (deviations from spec)

- [x] License: `MIT OR Apache-2.0`, copyright TPT Solutions
- [x] `tpt-hyle` uses layered features: bare `no_std`/no-alloc core -> `alloc` (serde/JSON) -> `std` (file/mmap backends)
- [x] `ChangeEvent`: replace single `vector_clock: u64` with `lamport: u64` + `node_id`; conflict order = `(lamport, node_id)`. (The spec's key-hash tie-break gives both peers the same input, so it can't separate them.)
- [x] MSRV 1.75, edition 2021
- [x] Repo: `github.com/tpt-solutions/tpt-hyle`

## Phase 0: Scaffold

- [x] Cargo workspace with 9 crates (compiles)
- [x] `LICENSE-MIT`, `LICENSE-APACHE`, README, .gitignore
- [x] CI workflow (fmt, clippy, test, no_std target, MSRV)
- [ ] `git init`, create GitHub repo, push
- [ ] Add `SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`
- [ ] Add `rustfmt.toml`, `clippy.toml`, `deny.toml` (cargo-deny: licenses + advisories)
- [ ] Set up `cargo-fuzz` / `cargo-kani` directories
- [ ] Per-crate README + crates.io metadata (keywords, categories)
- [ ] Reserve crate names on crates.io
- [ ] Decide `node_id` width (u64 vs 128-bit) and write ADR in `docs/`
- [ ] Write on-disk format spec in `docs/format.md` (magic, version, endianness, checksums)
- [ ] Finalize `ChangeEvent` layout (add key length/ref, checksum, node_id)

## Phase 1: tpt-hyle core (Year 1)

- [ ] `Storage` trait (byte-region abstraction: read/write/append/flush/len) so core has no file I/O
- [ ] In-memory fixed-buffer backend (no_alloc)
- [ ] File backend (`std`), optional mmap backend
- [ ] Segment headers + magic/version + per-record CRC / SHA-256
- [ ] State Segment append (key + value records)
- [ ] Sync Segment append (`ChangeEvent`), fixed-size records
- [ ] Atomic dual-append with crash recovery (torn-write detection, replay on open)
- [ ] `put` / `get` / `delete` (tombstones)
- [ ] Key index (hash -> offset); decide fixed-size table vs rebuild-on-open for no_alloc
- [ ] Hash-collision handling for `key_hash: u64` (store/compare full key)
- [ ] Sequential Sync Segment reader from offset (`read_since`)
- [ ] Lamport clock + node_id management
- [ ] Compaction: rewrite State, truncate Sync, crash-safe swap
- [ ] Define what compaction means for peers behind the truncation point (snapshot/resync protocol)
- [ ] `alloc` feature: JSON helpers (`put_json` / `get_json`), serde
- [ ] Error type (`thiserror` under `std`, hand-rolled in core)
- [ ] `tpt-hyle-test-utils`: in-memory store, proptest generators
- [ ] `tpt-hyle-cli`: `inspect`, `dump-sync-log`, `compact`, `verify`
- [ ] Corruption tests: truncated/flipped-bit files never panic
- [ ] Stress + property tests for integrity
- [ ] Benchmarks vs SQLite (KV write workload); target 10x
- [ ] CI builds core on `thumbv7em-none-eabihf` and `wasm32-unknown-unknown`

## Phase 2: tpt-rift + unified API (Year 2)

- [ ] `Transport` trait (the spec uses `Vec<ChangeEvent>`; decide if payload bytes ride along, since events only carry offsets)
- [ ] Wire format for deltas (events + payloads), versioned
- [ ] LWW element-set resolver, ordered by `(lamport, node_id)`
- [ ] Apply remote events into tpt-hyle without echoing them back (origin tracking)
- [ ] Per-peer sync cursors (last seen offset)
- [ ] Sync loop (timer / connectivity triggered), backoff
- [ ] Basic JSON CRDT (field-level LWW)
- [ ] Mock transport + two-instance offline-write convergence test
- [ ] `tpt-hyle-rift`: `open()`, `connect()`, background loop, `resume()`
- [ ] Property test: convergence regardless of delta order/duplication
- [ ] Examples in `examples/`

## Phase 3: Client-server (Year 3)

- [ ] `tpt-rift-transport-ws` client + server halves
- [ ] `tpt-rift-server` (axum, WebSocket endpoint, tpt-hyle storage)
- [ ] Authentication (decide: token / JWT / mTLS)
- [ ] Access control (per-namespace/key-prefix rules)
- [ ] Multi-client fan-out
- [ ] TLS guidance, rate limiting, size limits
- [ ] Docker image + deploy docs
- [ ] End-to-end demo app syncing to self-hosted server

## Phase 4: P2P + edge (Year 4)

- [ ] `tpt-rift-transport-webrtc` (signaling approach to decide)
- [ ] LAN discovery (mDNS / UDP broadcast)
- [ ] `tpt-rift-wasm`: wasm-bindgen API, IndexedDB `Storage` backend
- [ ] Browser demo: offline web app syncing P2P
- [ ] Kani harnesses: allocator bounds, segment-offset invariants through compaction
- [ ] CRDT determinism proof/tests (order-independence)

## Phase 5: Mobile + ecosystem (Year 5)

- [ ] 100% Kani coverage of core parser; publish proofs
- [ ] Swift (iOS) SDK via UniFFI or C ABI
- [ ] Kotlin (Android) SDK
- [ ] Nested JSON CRDT (concurrent edits in same object)
- [ ] Integrations: tpt-aion, tpt-archon, tpt-mnemosyne
- [ ] Docs site, benchmarks page, 1.0 stability policy

## Open questions

- [ ] Security model: encryption at rest? signed events? (spec is silent)
- [ ] Schema/versioning and migration story
- [ ] Is "100% formal verification" realistic with serde in the path? Scope proofs to the no_alloc core.
- [ ] Spec's `LWW-ES` vs JSON CRDT selection: per-key type tag or per-namespace?
- [ ] Deletion/tombstone GC across peers
