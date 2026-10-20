# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `connect(url)`: client half, for `ws://` and `wss://` URLs.
- `accept(listener)`: server half, one connection per call.
- Each binary message carries one encoded delta. A malformed message closes the link.

### Known limitations

- No authentication and no TLS. Use only on trusted links and in tests.
