# ADR 0011: LAN discovery by UDP broadcast

Status: accepted

## Context

Phase 4 asks for LAN discovery, with mDNS or UDP broadcast as options. Peers
on one network should find each other without a configured address.

## Decision

Use UDP broadcast on a fixed port (`47813`), with a 23-byte announcement that
carries a node id and a serving port. See [docs/discovery.md](../discovery.md).

Announcements are unauthenticated. They identify candidates. Authentication
still happens at connection time, with the bearer token from ADR 0006. So a
spoofed announcement can waste a connection attempt and nothing more.

## Why not mDNS

mDNS needs a responder, a service-type registration, and a parser for
multicast DNS records. That is a second protocol stack, and this feature needs
only "who is here and where". Broadcast is simpler and works on every platform
that has UDP. It stops at the subnet, which is the scope this feature targets.

## Consequences

- Discovery works within one broadcast domain only.
- Node ids are visible to the LAN. Anyone who sees an announcement learns the
  id, so ids should not be treated as secrets.
- Tests use unicast to loopback. Broadcast on the loopback interface is not
  reliable on every platform, so the broadcast path itself is not tested here.
