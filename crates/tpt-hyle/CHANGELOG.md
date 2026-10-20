# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Append-only key-value store with `put`, `get` and `delete`.
- Batches that commit with one fsync. A torn tail is truncated on open.
- Sync cursors (`events_since`) and a Lamport clock for replication.
- Crash recovery, compaction (in memory and file), and an in-memory key index.
- Encryption at rest: XChaCha20-Poly1305 sealed frames, with passphrase keys
  derived by Argon2id (feature `std`).
- Per-event Ed25519 signatures, with `public_key()`.
- `put_json` and `get_json` (feature `json`).
- Tombstones are collected once every client has acknowledged them (ADR 0014).
- Feature flags: `alloc`, `std` (default) and `json`. The core builds `no_std`.

### Changed

- The store is one encrypted file, on-disk format v3.
- Every event is signed by the issuing node's Ed25519 key, on-disk format v4.
