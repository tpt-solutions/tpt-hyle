# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- LAN discovery for tpt-rift peers. Each node broadcasts a 23-byte UDP
  announcement with its node id and serving port, and a listener collects them.
