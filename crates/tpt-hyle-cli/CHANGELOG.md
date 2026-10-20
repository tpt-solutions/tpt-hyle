# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `inspect`: node id, clock, generation, write and tombstone counts, live keys,
  file size and public key.
- `dump-sync-log`: prints every write in order, with `--json` output.
- `verify`: reads and authenticates every frame.
- `public-key`: prints the Ed25519 public key.
- `compact`: drops superseded values and starts a new generation.
- The passphrase is read from `TPT_HYLE_PASSPHRASE`, so it stays out of shell
  history.
