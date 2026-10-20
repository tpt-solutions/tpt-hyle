# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `open(path, node_id)`, which opens a file-backed replica.
- `connect(&mut db, peer_node, transport)`, which starts a sync session with one
  peer.
- `Connection::resume()`, which runs one sync round. A stale peer position is
  reset and retried automatically.
- `two_replicas` example.
