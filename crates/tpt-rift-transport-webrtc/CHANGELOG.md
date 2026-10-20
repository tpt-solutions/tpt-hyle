# Changelog

All notable changes to this crate are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- WebRTC peer-to-peer `Transport`. One data channel carries one encoded delta
  per message.
- Offer and answer exchange (`create_offer`, `accept`, `Offer::complete`). The
  caller carries the session descriptions over any signaling path.
- STUN and TURN configuration (`stun_urls`, `turn`) for peers behind NAT.
- Loopback and STUN tests.
