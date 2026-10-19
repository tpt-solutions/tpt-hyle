# ADR 0005: Batched commits and commit records

Status: accepted

## Context

ADR 0002 left one question open. A write was made durable one record at a
time, which needs two fsyncs per put, and SQLite needs one. A batch API would
close the gap, but flushing sync records in order is not enough. A crash can
persist record `k+1` without record `k`, and recovery then sees a damaged
record followed by valid ones.

## Decision

Every write belongs to a batch. A batch is closed by a **commit record**, a
sync record with the commit flag set. Commit order is:

1. Flush the State Segment.
2. Flush the Sync Segment, which makes the batch's data records durable.
3. Append the commit record, then flush the Sync Segment.

Recovery keeps everything up to the last valid commit record. Records after
it are discarded, because they belong to a batch that never committed. A
damaged record before the last commit record is still reported as `Corrupt`,
since it was committed.

The commit record carries the clock. A batch that holds no events can still
persist the clock by committing with `commit_forced`. Import and compaction
use that.

A dropped, uncommitted batch rolls back by truncating both segments to their
lengths at the start of the batch.

## Why this layout

- **Atomic batches.** A crash mid-batch loses the whole batch, never part of
  it. The commit record is the only point of commitment.
- **The ordering is kept.** Step 2 persists every data record before the
  commit record can be written, so a persisted commit record never refers to
  missing data.
- **Single writes stay correct.** A put is a batch of one. It costs three
  flushes instead of two, because the commit record must not persist ahead
  of the data it closes. That is the price of the framing, and it is paid
  once per batch, not once per record.

## Alternatives considered

- **Per-record flags on the last record only.** Smaller, but a crash can
  still persist the closing record without its predecessors. That is the
  exact failure this design exists to prevent.
- **Hash chaining.** Detects gaps, but gives no defined rollback point.
- **Keeping per-put durability and adding a separate batch API.** Two
  durability rules are harder to reason about than one.

## Consequences

- `FORMAT_VERSION` is 2. Version 1 stores cannot be opened.
- Sync logs hold one extra 56-byte record per batch. Compaction emits one
  batch, so it adds one.
- Replication exports skip commit records. The feed contains write events
  only.
- Benchmarks: a batch of N writes costs three flushes in total. Per-put
  durability costs three flushes per write. See `bench/`.
