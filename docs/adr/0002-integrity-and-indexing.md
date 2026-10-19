# ADR 0002: Record integrity, recovery, and key lookup

Status: accepted

## Context

The Phase 1 TODO leaves four questions open: which checksum to use per
record, how to recover from a crash, how the key index is kept, and which
error type to use. The core must stay `no_std` with no heap on its main path.

## Decisions

### 1. Checksum: CRC-32C, hand-rolled

Every state and sync record carries a CRC-32C. It is a table-driven
implementation in `checksum.rs`, built by a `const fn`, with no dependency.

SHA-256 was considered. It is cryptographic, which gives tamper evidence
rather than just corruption detection. That is the wrong tool here: the
threat is torn writes and bit rot, and CRC-32C detects those with a much
smaller cost. The security model (encryption at rest, signed events) is still
an open question in [TODO.md](../../TODO.md). When it is answered, a MAC or
signature can sit beside the CRC without changing this layer.

### 2. Recovery: truncate a torn tail, refuse mid-log damage

Writes are ordered state-then-sync, each followed by a flush. So a crash can
leave at most one partial record, at the end. Recovery therefore:

- truncates a bad or partial **final** record, and
- returns `Corrupt` if a bad record has a full record after it.

The second rule matters. Silently truncating in the middle would discard
committed writes with no sign that anything went wrong.

### 3. Key lookup: linear reverse scan for now

Two options were on the table: a fixed-size hash table (`no_alloc`) or an
index rebuilt on open (`alloc`). Neither is built yet. `get` scans the Sync
Segment newest-first, comparing `key_hash` and then the full key. This is
O(n) per read.

The reasoning is that the benchmark target (10x over SQLite) is what should
drive the index design. Building an index before there are numbers would be
guessing. The `key_hash` field and the state-record key are there so that
an index can be added later without a format change. The "Key index" TODO
item stays open until a benchmark justifies one.

### 4. Errors: one hand-rolled enum

`Error` is a single `#[non_exhaustive]` enum with a manual `Display`. Under
`std` it implements `std::error::Error`. This avoids the `thiserror` derive,
which needs `std` and so does not fit the core. The TODO's suggestion of
`thiserror` under `std` is not followed.

## Consequences

- Each record has a fixed, documented integrity check. See
  [docs/format.md](../format.md).
- Write amplification is one extra flush per put. That is the cost of the
  ordering guarantee.
- Lookup cost grows with the live log length until a key index exists.

## Batched commits (resolved by ADR 0005)

The benchmark in `bench/` puts each write as its own durable commit. That
costs two fsyncs per put, one per segment, so file-backed writes run at half
SQLite's throughput. A batch API that flushes once would fix this, but only
with a batch-framing decision first. Flushing records in order is not enough.
A crash can persist sync record `k+1` without record `k`. Recovery would
then see a damaged record followed by valid ones and refuse the store. A
batch needs a commit marker or per-batch framing so a partial batch is
dropped as a whole. That format change is not made yet.
