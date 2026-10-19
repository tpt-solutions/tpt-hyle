# ADR 0004: Replication semantics

Status: accepted

## Context

Phase 2 connects replicas. Each one writes offline, so the rules for merging
concurrent writes need to be fixed before any transport exists. The design
also has to tolerate lossy or repeated delivery.

## Decisions

### 1. Deltas carry payloads

The spec's `Transport` moves `Vec<ChangeEvent>`. A `ChangeEvent` holds an
offset into the sender's State Segment, which is useless to the receiver. So
a delta carries each change's key, value, lamport, node id and tombstone flag.
See [docs/wire.md](../wire.md).

### 2. Last writer wins by `(lamport, node_id)`

Each key has one winning change: the one with the greatest `(lamport,
node_id)`. A tombstone can win, which deletes the key. Among concurrent writes
the lamport decides. Equal lamports are broken by node id, so the order is
total, and the result does not depend on who receives what first.

Taking the maximum is commutative, associative and idempotent. Any replica
that has received the same set of changes therefore reaches the same state,
whatever the order and however many duplicates there were.

The convergence test in `crates/tpt-hyle-test-utils/tests/convergence.rs`
checks this with random writes, random orders and duplicates. It was
mutation-checked: replacing the comparison with "always apply" makes it fail.

### 3. The last appended event is the winner

Import appends only changes that beat the current latest event for their key.
So the newest entry in the local log is always the winner. That keeps `get`,
which scans newest-first, consistent with the LWW rule. Compaction keeps the
last event per key, so it preserves the same property.

### 4. The clock covers what it has seen

Applying a remote change advances the local clock to its lamport. A later
local write is therefore always ordered after anything the replica has
received.

### 5. Origin tracking avoids echo, not forwarding

When exporting to a peer, events whose origin is that peer are omitted. A
replica's own writes and writes it forwarded from others are still sent. So
a line topology `A - B - C` delivers A's writes to C through B, and nothing
bounces back to A. Duplicates that arrive by other paths are harmless.

### 6. Cursors are per peer, and advance after delivery

The sender keeps `(generation, offset)` for each peer. The cursor advances
only after a successful send, so a failed send is retried from the same
place. Advancing past events that were filtered out, because they came from
the peer, is correct: the peer already has them.

### 7. Compaction forces a resend, not a merge

A cursor from an older generation is refused with `StaleGeneration`. The
peer resets to the start and receives the whole compacted log again. This is
safe for two reasons. The compacted log holds the winning change for each
key, so it is enough to converge. Import is idempotent, so re-sending
changes is harmless.

The one thing a resend cannot recover is the history of superseded values.
Nothing needs it for convergence, so that is acceptable.

## Consequences

- Concurrent writes to the same key lose one of them, as the spec says. This
  is the last-writer-wins trade-off. Field-level merging is the JSON CRDT,
  which is still open.
- Tombstones are kept by compaction, so deletes still reach peers that never
  saw the value. Garbage-collecting them remains an open question.
- The clock is a logical counter, not wall time. Nodes are not synchronized
  to real time, so "last" means last in causal order as far as the clock can
  tell, not last in wall time. User-facing docs should say so.
