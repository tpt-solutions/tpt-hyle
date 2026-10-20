# ADR 0008: Key index

> **Note.** The index is unchanged in principle. Keys are now hashed with FNV-1a, and each candidate is checked against the full key. The index is rebuilt from the file on open. See [docs/format.md](../format.md).


Status: accepted

## Context

ADR 0002 deferred a key index until benchmarks showed that reads needed one.
They did. `get` scanned the sync log newest-first, so its cost grew with the
log: 86 µs at 1,000 keys, 868 µs at 10,000, and 4.4 ms at 50,000 keys.

## Decision

With the `alloc` feature, the store keeps an in-memory index from key hash to
the sync offsets of that hash's data records, oldest first. A lookup walks the
short list for the key's hash, newest first. Each candidate is checked against
the full key, so a hash collision cannot return the wrong value.

- **Rebuilt on open.** Recovery already reads every committed record, so it
  adds each data record to the index as it goes.
- **Maintained on write.** Each appended data record is added as it is written.
- **Retracted on rollback.** A dropped uncommitted batch removes its entries.
- **Not on disk.** The format is unchanged. The index is derived state.
- **Without `alloc`.** The no_std core still scans. A fixed-size index for
  that path is a separate design, and nothing needs it yet.

Reads on a poisoned store are refused. A failed commit can leave records in
the log that a reopen would discard, and an index would expose them in the
meantime.

## Results

| Keys in log | Before (scan) | After (index) |
|---|---|---|
| 1,000 | 86 µs per get | 0.5 µs |
| 10,000 | 868 µs per get | 0.8 µs |
| 50,000 | 4,425 µs per get | 1.0 µs |

## Consequences

- Memory is about one offset per data record, roughly 8 bytes plus map overhead.
  Compaction shrinks the log and so the index.
- Opening a store is still linear in log length, because recovery verifies
  every record. An index does not change that.
- `live_entries` still scans the whole log. A prefix index would be the next step
  if documents become large.
