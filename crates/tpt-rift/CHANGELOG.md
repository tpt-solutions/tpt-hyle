# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `Replica`, which exports the changes a peer has not seen and imports changes
  with last-writer-wins by `(lamport, node_id)`.
- `Transport` trait for network and in-process links.
- `MockTransport`, an in-process link for tests and examples.
- `sync_round`, `PeerLink` and `Backoff`: one round of sync, per-peer state, and
  a retry policy.
