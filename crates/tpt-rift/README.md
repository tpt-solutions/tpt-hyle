# tpt-rift

CRDT sync engine that replicates directly from the `tpt-hyle` Sync Segment.

- `Replica`: exports the changes a peer has not seen, and imports changes
  with last-writer-wins by `(lamport, node_id)`.
- `Transport`: the trait a network or in-process link implements.
- `MockTransport`: an in-process link for tests and examples.
- `sync_round`, `PeerLink`, `Backoff`: one round of sync, per-peer state, and
  a retry policy.

Wire format: [docs/wire.md](../../docs/wire.md). Semantics:
[ADR 0004](../../docs/adr/0004-replication-semantics.md).

Status: the Phase 2 core is implemented and tested. Network transports are
Phase 3.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
