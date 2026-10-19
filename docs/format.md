# On-disk format (version 2)

This describes the bytes `tpt-hyle` writes. The code in
`crates/tpt-hyle/src/format.rs` and `crates/tpt-hyle/src/store.rs` is the
reference implementation. If the two disagree, fix whichever is wrong and
bump `FORMAT_VERSION` for any change to existing bytes.

## Conventions

- All integers are **little-endian**.
- Offsets are **absolute byte offsets from the start of a segment**, header
  included.
- Checksums are **CRC-32C** (Castagnoli, polynomial `0x1EDC6F41`, reflected).
  The check value of `"123456789"` is `0xE3069283`.
- `key_hash` is **FNV-1a 64** over the key bytes. It is a filter, not an
  identity. Full keys are always compared.

## Store layout

A store is two append-only segments:

| Segment | Magic  | Contents                                  |
|---------|--------|-------------------------------------------|
| State   | `HYST` | Variable-length records: key and value    |
| Sync    | `HYSY` | Fixed 56-byte `ChangeEvent` records       |

Each segment starts with a 32-byte header. The `node_id` is the same in both
headers.

### Segment header (32 bytes)

| Offset | Size | Field          | Value                              |
|--------|------|----------------|------------------------------------|
| 0      | 4    | magic          | `HYST` or `HYSY`                   |
| 4      | 2    | version        | `2`                                |
| 6      | 2    | reserved       | `0`                                |
| 8      | 16   | node_id        | `u128`, see [ADR 0001](adr/0001-node-id-width.md) |
| 24     | 4    | generation     | `u32`. Starts at `0`, incremented by each compaction |
| 28     | 4    | crc32c         | CRC-32C of bytes `0..28`           |

A reader rejects the segment if the magic, CRC, version, or reserved bytes
are wrong. The State and Sync segments of one store must have the same
`generation`, or the store refuses to open.

### State record (variable length)

| Offset | Size      | Field      | Notes                                   |
|--------|-----------|------------|-----------------------------------------|
| 0      | 4         | crc32c     | CRC-32C of bytes `4..10+key_len+value_len` |
| 4      | 2         | key_len    | `1..=65535`                             |
| 6      | 4         | value_len  | `0..=u32::MAX`. `0` for tombstones      |
| 10     | key_len   | key        |                                         |
| 10+k   | value_len | value      |                                         |

The key lives here, not in the sync record. That is what lets a hash
collision be told apart from a real match.

### Sync record (fixed 56 bytes)

| Offset | Size | Field          | Notes                                              |
|--------|------|----------------|----------------------------------------------------|
| 0      | 4    | crc32c         | CRC-32C of bytes `4..56`                           |
| 4      | 8    | key_hash       | FNV-1a 64 of the key                               |
| 12     | 8    | lamport        | Lamport timestamp of the write                     |
| 20     | 16   | node_id        | Node that issued the write                         |
| 36     | 8    | payload_offset | Offset of the state record in the State Segment    |
| 44     | 4    | payload_len    | Value length. Must equal the state record's        |
| 48     | 2    | key_len        | Must equal the state record's                      |
| 50     | 1    | flags          | bit 0 = tombstone, bit 1 = commit record. Other bits must be `0` |
| 51     | 5    | padding        | `0`                                                |

Total 56 bytes. Conflict order is `(lamport, node_id)`. This replaces the
spec's single `vector_clock` field ([TODO.md](../TODO.md), decisions).

Logical `ChangeEvent` in Rust:

```rust
pub struct ChangeEvent {
    pub key_hash: u64,
    pub key_len: u16,
    pub lamport: u64,
    pub node_id: u128,
    pub payload_offset: u64,
    pub payload_len: u32,
    pub tombstone: bool,
}
```

## File layout

A file-backed store called `<base>` uses three paths:

| Path                        | Role                                              |
|-----------------------------|---------------------------------------------------|
| `<base>.sync`               | The current Sync Segment. Its header names the generation. |
| `<base>.state.<generation>` | The State Segment of that generation.             |
| `<base>.sync.tmp`           | Scratch file during compaction. Never valid on its own. |

Only the files of the current generation are live. On open, files for other
generations and any `.sync.tmp` are deleted.

## Operations

**Batches.** Every write belongs to a batch. A batch appends its state and
sync records without flushing. `commit` then writes, in order:

1. Flush the State Segment. Every state record of the batch is durable.
2. Flush the Sync Segment. Every data record of the batch is durable.
3. Append one **commit record** to the Sync Segment, then flush it.

A commit record closes a batch. It carries the clock at commit time. A batch
with no writes commits nothing, unless it is forced, which writes a commit
record so the clock survives a restart.

A batch dropped without commit is rolled back: both segments are truncated to
their lengths when the batch started.

**put(key, value)** is a batch of one.

**delete(key)** is a tombstone written in a batch. It writes nothing and
returns `false` if the key is absent.

**get(key)** scans the Sync Segment newest-first. The first event whose
`key_hash` and `key_len` match, and whose state record has the same key, is
the answer. A tombstone means the key is absent.

**Sync cursor.** A cursor is an absolute offset in the Sync Segment, always
`32 + n*56`. A peer that has processed events up to `c` resumes at `c`. The
cursor returned with each event is the offset *after* that event.

A cursor is only meaningful together with the generation it was taken from.
Compaction changes the log, so a peer must store `(generation, cursor)`. If
the generation differs, the peer must resynchronize from the header.

**Compaction** produces the next generation. It keeps the last event for each
key, tombstones included, in its original order. Each kept event keeps its
original `lamport` and `node_id`. The new store's clock is the old clock, so
it never moves backwards. Superseded values and events are dropped.

Tombstones are kept, so a deletion still reaches peers that never saw the
live value. Removing old tombstones is an open question
(see [TODO.md](../TODO.md)).

The file swap is in [ADR 0003](adr/0003-compaction-and-generations.md). Its
commit point is one atomic rename of `<base>.sync.tmp` over `<base>.sync`.

## Recovery on open

Recovery works in two passes over the Sync Segment:

1. Find the **last commit record** whose CRC is valid. If there is none, the
   store is empty and both segments are truncated to their headers.
2. Check every record from the header up to that commit record. A damaged
   record there is committed data, so open fails with `Corrupt`. A data
   record is valid only if its state record is intact (in bounds, CRC
   matches, lengths agree, and `FNV-1a(key) == key_hash`).
3. Truncate the Sync Segment to just after that commit record, and the State
   Segment to the end of the last data record it covers. Everything after the
   commit record belongs to a batch that never committed. Torn records and
   orphaned state bytes are removed.
4. The clock resumes from the largest `lamport` seen in a valid data or
   commit record.
5. Before any of this, the store checks that the State and Sync segments
   have the same `generation`. If they differ, it refuses to open.

Consequence: a torn or unfinished batch disappears whole. Damage to any
committed record is reported, never silently dropped.

Reads also verify the state record checksum. A value damaged after open
returns `Corrupt` from `get`.

## Limits

| Item       | Limit                                                   |
|------------|---------------------------------------------------------|
| Key        | 1 to 65535 bytes                                        |
| Value      | 0 to `u32::MAX` bytes, and no larger than the backend allows |
| Node id    | `u128`                                                  |
| Lamport    | `u64`. Writes fail with `LamportOverflow` at the top    |
| Segment    | `u64` bytes                                             |

## Versioning

`FORMAT_VERSION = 2`. A reader rejects any other version with `Corrupt`.

Version 2 added commit records and batches. Version 1 stores cannot be opened.
Nothing has been released yet, so no migration tool exists. Once a release
ships, any further change needs a migration path.
Migration tooling is not written yet. See the open question on schema and
migration in [TODO.md](../TODO.md).
