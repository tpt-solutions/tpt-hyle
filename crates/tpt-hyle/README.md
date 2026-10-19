# tpt-hyle

A sync-native, append-only key-value store. Every write lands in two
segments: the value in the **State Segment**, and a fixed 56-byte
`ChangeEvent` in the **Sync Segment**. The Sync Segment is the replication
feed, so no change-tracking layer is needed.

- `no_std`, with a heap-free core path (`FixedStorage`).
- `alloc` adds the growable in-memory `VecStorage`.
- `std` adds file-backed storage (`Hyle::open_file`).
- CRC-32C on every record. Torn tails are truncated on open. Mid-log damage
  is reported, not silently dropped.

```rust
use tpt_hyle::{FixedStorage, Hyle};

let mut db = Hyle::open(FixedStorage::<4096>::new(), FixedStorage::<4096>::new(), 1)?;
db.put(b"user:101", b"alice")?;

let mut buf = [0u8; 16];
let n = db.get(b"user:101", &mut buf)?.unwrap();
assert_eq!(&buf[..n], b"alice");
# Ok::<(), tpt_hyle::Error>(())
```

## Status

Phase 1 core: `put`, `get`, `delete`, sync-log cursors (`events_since`),
Lamport clock, and crash recovery are implemented. Not yet implemented:
compaction, a key index, `mmap`, and `put_json`/`get_json`.

## Documentation

- [docs/format.md](../../docs/format.md): the on-disk byte layout.
- [docs/adr/](../../docs/adr/): design decisions and their reasons.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
