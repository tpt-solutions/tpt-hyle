# tpt-hyle

A sync-native, append-only key-value store. The write log is the replication
feed, so no change-tracking layer is needed. A store is one encrypted file.
Every write is an Ed25519-signed, XChaCha20-Poly1305-sealed frame.

- `no_std`, with a heap-free core path (`FixedStorage`).
- `alloc` adds the growable in-memory `VecStorage` and the engine.
- `std` adds the file backend (`Hyle::open_file`) and passphrase keys (Argon2id).
- Batches commit with one fsync. A torn tail is truncated on open. Damage inside
  committed data is reported, not silently dropped.

```rust
use tpt_hyle::Hyle;

let mut db = Hyle::open_file("notes.db", 1, b"a passphrase")?;
db.put(b"user:101", b"alice")?;

let mut buf = [0u8; 16];
let n = db.get(b"user:101", &mut buf)?.unwrap();
assert_eq!(&buf[..n], b"alice");
# Ok::<(), Box<dyn std::error::Error>>(())
```

The same passphrase opens the store again. A wrong passphrase returns
`Error::WrongKey` and leaves the file untouched. Losing the passphrase loses the
data.

## Status

Implemented: `put`, `get`, `delete`, batches, sync cursors (`events_since`),
Lamport clock, crash recovery, compaction (in memory and file), key index,
`put_json`/`get_json` (feature `json`), encryption at rest, and per-event
Ed25519 signatures with `public_key()`.

Not implemented: `mmap`, and a way to rotate a node's key without rewriting the
store.

## Documentation

- [docs/format.md](../../docs/format.md): the on-disk byte layout.
- [docs/adr/](../../docs/adr/): design decisions and their reasons. ADR 0013
  (encryption and single file), ADR 0014 (tombstone collection), and ADR 0015
  (signed events) are the current ones.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
