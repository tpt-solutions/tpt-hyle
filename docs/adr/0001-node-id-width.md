# ADR 0001: Node id width

Status: accepted

## Context

Every write carries the id of the node that issued it. Conflict order is
`(lamport, node_id)`, so ids must not collide. The choice is between a 64-bit
and a 128-bit id.

## Decision

Use a **128-bit** node id (`u128`), stored in each segment header and in each
sync record.

## Reasoning

- **Collision risk.** Ids are picked without coordination, usually at random.
  A 64-bit id reaches a 50% chance of some collision near 2^32 nodes. A
  128-bit id keeps that risk negligible at any realistic fleet size.
- **Interop.** A 128-bit value fits a UUID or a random 16-byte identifier
  without truncation, so an application can use its existing identifiers.
- **Cost.** Eight extra bytes per sync record. A 64-bit id would give a
  48-byte record. The 128-bit id gives 56 bytes, and the 5 padding bytes
  that remain are reserved for future flags.
- **no_std.** `u128` is in core, so no heap or dependency is needed.

## Consequences

- The sync record layout in [docs/format.md](../format.md) is fixed at 56
  bytes.
- A store is bound to one node id. Opening it with a different id fails with
  `NodeIdMismatch`. Multi-node data arrives as remote events (Phase 2), so
  this is not a limit on replication.
