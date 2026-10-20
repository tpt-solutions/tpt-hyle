# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- WebAssembly `Store`, an in-memory encrypted replica that does no I/O. It
  exposes `new`, `load`, `put`, `get`, `delete`, `lamport`, `export`, `import`,
  `save`, `add_peer` and `public_key`.
- IndexedDB persistence in `web/persistence.js`, tested under Node with
  fake-indexeddb.
- Node example (`two_stores.cjs`) and a Node test suite.
- Writes are signed by the store's node. `add_peer` makes `import` refuse changes
  from unlisted nodes or with bad signatures.

### Known limitations

- No browser demo page yet.
