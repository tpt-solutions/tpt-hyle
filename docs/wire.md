# Wire format: deltas (version 3)

A **delta** is what one replica sends another. It carries the changes that
peer has not yet seen, with their keys and values, and, optionally, the two
control fields that tombstone collection uses. Offsets are not sent for
changes, because an offset is only meaningful inside the log that produced it.
The control fields carry hub log offsets, and say what they refer to.

The reference implementation is `crates/tpt-rift/src/wire.rs`. The protocol
built on it is in [ADR 0014](adr/0014-tombstone-collection.md).

## Conventions

- All integers are **little-endian**.
- Decoding never panics. Every length is checked against the bytes that
  actually arrived before it is used.

## Delta layout

| Offset | Size | Field        | Notes                                        |
|--------|------|--------------|----------------------------------------------|
| 0      | 4    | magic        | `RIFT`                                       |
| 4      | 1    | version      | `4`                                          |
| 5      | 4    | generation   | Generation of the sender's log at export     |
| 9      | 4    | count        | Number of changes. At most `1 << 20`         |
| 13     | 1    | control      | Bit 0: `caught_up` present. Bit 1: `ack` present. Other bits must be `0` |
| 14     | 8    | caught_up    | Only if control bit 0. Hub log offset       |
| …      | 80   | ack          | Only if control bit 1: `offset` (8), `lamport` (8), `signature` (64) |
| …      | ...  | changes      | `count` entries, in the sender's log order   |

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
| 64        | signature  | Ed25519 signature by `node_id` over the event      |

## Signatures

Each change carries its origin's Ed25519 signature. A receiver with a
directory of node keys checks every change in a delta before applying any of
them. A change from an unlisted node, or with a bad signature, refuses the whole
delta. The signed message is defined in [docs/format.md](format.md#signatures).
A relay forwards a change with its origin's signature, unchanged.

A receiver without a directory does not check signatures. It applies changes as
before.

## Control fields

- **`caught_up`** is sent by the hub. It says that the hub has sent this client
  every change in its log up to that offset, in the delta's generation.
- **`ack`** is sent by a client, and it is signed by that client's node key over
  the generation, offset and lamport (`tpt_hyle::verify_ack`). A hub refuses an
  ack that does not verify.
  It says that the client has applied the hub
  up to the `caught_up` offset it last received, and has pushed every write it
  made with a lamport at or below `ack.lamport`. The client reads
  `ack.lamport` before its export, and sends the ack only after an export that
  reached the end of its log.

A delta may carry changes and a control field together. The receiver applies
the changes first, then the control field.

## Rules

A decoder rejects a delta that has:

- the wrong magic or an unknown version,
- a `count` larger than `1 << 20`, or one that cannot fit in the bytes that
  remain. This is checked before any allocation,
- truncated content, or trailing bytes after the last change,
- unknown control bits, unknown flag bits, an empty key, or a tombstone with a
  value.

## Semantics

Receivers apply changes with last-writer-wins by `(lamport, node_id)`. See
[ADR 0004](adr/0004-replication-semantics.md). Deltas may be lost, delayed,
duplicated, or reordered. Importing the same delta twice has no effect.

The `generation` field is informational for changes. Cursors are tracked by the
sender per peer, not by the receiver. For `caught_up` and `ack`, the generation
is what they refer to, and a receiver ignores them if it is in another one.

## Versioning

Any change to the bytes above bumps `WIRE_VERSION`. Receivers reject versions
they do not know. Versions 1, 2 and 3 are not accepted by version 4 code, and there is no
negotiation. Peers must be upgraded together.
