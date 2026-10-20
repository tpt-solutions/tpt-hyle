# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Reference sync server: a hub that stores data and passes changes between
  authenticated WebSocket clients.
- Bearer tokens, stored only as SHA-256 hashes. Each token is bound to one node id.
- Key-prefix read and write permissions per token.
- Message size limit and a per-connection token-bucket rate limit.
- `new-token`, `serve` and `collect` commands.
- Container image definition (Dockerfile at the repository root).
- `notes` example: a command-line notes client (`put`, `get`, `sync`) for the
  end-to-end demo in `docs/server.md`.

### Known limitations

- Hot config reload is not built. TLS is expected from a reverse proxy.
