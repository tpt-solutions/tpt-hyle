# ADR 0014: Tombstone collection with client acknowledgements

Status: accepted

## Context

A delete is a tombstone. Tombstones are kept forever, so the hub's log grows
with every key ever deleted. Compaction keeps the latest event per key,
including tombstones, so a deleted key stays deleted on any peer that has not
yet seen the delete.

A tombstone can be dropped only when no peer can still bring back the value it
replaced. That happens if a peer later pushes an older write for the key, and
the hub has nothing newer to refuse it. The hub cannot know, from its own log,
whether every peer has seen the delete.

## Decision

Clients acknowledge the hub's log. Compaction drops a tombstone only when every
configured client has acknowledged past it.

### The handshake

1. **Marker.** When the hub has sent a client everything in its log, it sends
   a `caught_up` marker: the log end offset, in its current generation.
2. **Ack.** The client acknowledges the marker with `(offset, lamport)` once
   both hold:
   - it has applied every change before the marker, and
   - its last export reached the end of its own log, so every write it has
     made is on the hub. The `lamport` is the clock read before that export.
3. The hub records the ack per client token, with the generation. It refuses an
   ack that names another generation, lies past its log or clock, or moves
   backwards. It saves the acks to `<store>.acks`, atomically.

### Collection

`tpt-rift-server collect <config>`, run while the server is stopped, computes a
bound over every configured client:

- `bound.offset` is the lowest acked offset, and
- `bound.lamport` is the lowest acked clock.

A tombstone is dropped when its end offset is at or below `bound.offset` and its
lamport is at or below `bound.lamport`. Only the latest event per key is kept,
as in ordinary compaction, so the dropped tombstone takes its key with it.

Compaction starts a new generation. Clients then resync from the start, and
acks from the old generation no longer count.

### Replay filter

Once every client has acked in the current generation, the hub drops incoming
changes with a lamport at or below the lowest acked clock. Those changes are
replays. The client sent them before it acked, and the hub already has them, or
has superseded or collected them.

## Why it is safe

Three facts together:

- **The tombstone reached each client.** A client's ack offset is past the
  tombstone, so the client applied it. The client's value for the key is now
  the delete, not the older value.
- **Nothing older is still unpushed.** The ack is sent only after a complete
  export, so every write the client made with a lamport at or below its ack
  clock is already on the hub.
- **Nothing older can appear later.** Lamport clocks only move forward. After
  the ack, the client's clock is at least the ack clock, so every later write
  has a lamport above the bound. It loses to the tombstone, and so does any
  replay that the filter has not caught.

## Consequences

- **A client that never connects blocks collection for the whole store.** No
  tombstone is collected until every configured client has acked. This is
  deliberate. A client that is offline may still hold older writes. Remove a
  token from the config only when its client is gone for good, and then accept
  that its unpushed writes are gone too.
- **Collection needs the server stopped.** It rewrites the store file.
- **Collection starts a new generation.** Every client does a full resync on
  its next connection. The cost is bandwidth, once per collection.
- **Trust.** The hub believes a client's ack. A token holder who lies about its
  ack can make the hub drop a tombstone that client has not applied. The hub
  caps an ack at its own log end and clock, but a lie within those bounds is
  not detected. Signed events (planned) would let the hub check a client's
  clock against its signatures.
- **Wire version 2.** `Delta` carries the optional `caught_up` and `ack`. A
  version 1 peer is refused.
- **The export cursor now moves to the log end** when the export runs out of
  events, instead of stopping at the last event. This lets a caught-up check
  compare against the true end. Commit frames carry no events, so skipping
  them changes nothing that a peer can see.

## Alternatives rejected

- **Time-based expiry.** A tombstone older than N days is dropped. A peer offline
  for longer than N days can then resurrect the value. Wrong by design.
- **Lamport-only acks.** A client's clock says how far it has got, not whether
  it has applied a particular tombstone. A late tombstone with a low lamport
  would be missed, so the ack also carries the log offset.
- **Acks inside the store.** Acks would replicate to every client, and a client
  could read another client's progress. A sidecar file stays with the hub.
- **Collecting on the client too.** Only the hub holds the full set of acks, so
  only the hub can decide. Clients keep their tombstones until they resync.

## Related

- [[0003-compaction-and-generations]] — the generation rule that resets acks.
- [[0004-replication-semantics]] — last-writer-wins, which makes the replay
  filter sound.
- [[0013-encryption-and-single-file]] — the file that compaction rewrites.
