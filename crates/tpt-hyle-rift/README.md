# tpt-hyle-rift

Unified API that combines `tpt-hyle` storage with `tpt-rift` sync.

- `open(path, node_id)`: a file-backed replica.
- `connect(&mut db, peer_node, transport)`: a sync session with one peer.
- `Connection::resume()`: one sync round. A stale peer position is reset and
  retried automatically.

```sh
cargo run -p tpt-hyle-rift --example two_replicas -- <scratch-dir>
```

Status: `open`, `connect` and `resume` are implemented. A background loop
with timers is not built yet. Callers schedule rounds, and
`tpt_rift::Backoff` gives the retry delays.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
