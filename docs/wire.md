# Wire format: deltas (version 1)

A **delta** is what one replica sends another. It carries the changes that
peer has not yet seen, with their keys and values. Offsets are not sent,
because an offset is only meaningful inside the log that produced it.

The reference implementation is `crates/tpt-rift/src/wire.rs`.

## Conventions

- All integers are **little-endian**.
- Decoding never panics. Every length is checked against the bytes that
  actually arrived before it is used.

## Delta layout

| Offset | Size | Field        | Notes                                        |
|--------|------|--------------|----------------------------------------------|
| 0      | 4    | magic        | `RIFT`                                       |
| 4      | 1    | version      | `1`                                          |
| 5      | 4    | generation   | Generation of the sender's log at export     |
| 9      | 4    | count        | Number of changes. At most `1 << 20`         |
| 13     | ...  | changes      | `count` entries, in the sender's log order   |

Each change:

| Size      | Field      | Notes                                              |
|-----------|------------|----------------------------------------------------|
| 8         | lamport    | Lamport timestamp of the write                     |
| 16        | node_id    | Node that issued the write                         |
| 1         | flags      | bit 0 = tombstone. Other bits must be `0`          |
| 2         | key_len    | Must be at least 1                                 |
| 4         | value_len  | Must be `0` for a tombstone                        |
| key_len   | key        |                                                    |
| value_len | value      |                                                    |

## Rules

A decoder rejects a delta that has:

- the wrong magic or an unknown version,
- a `count` larger than `1 << 20`, or one that cannot fit in the bytes that
  remain. This is checked before any allocation,
- truncated content, or trailing bytes after the last change,
- unknown flag bits, an empty key, or a tombstone with a value.

## Semantics

Receivers apply changes with last-writer-wins by `(lamport, node_id)`. See
[ADR 0004](adr/0004-replication-semantics.md). Deltas may be lost, delayed,
duplicated, or reordered. Importing the same delta twice has no effect.

The `generation` field is informational. Cursors are tracked by the sender
per peer, not by the receiver.

## Versioning

Any change to the bytes above bumps `WIRE_VERSION`. Receivers reject versions
they do not know. There is no negotiation yet. That belongs with the Phase 3
transports.
