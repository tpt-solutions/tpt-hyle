# tpt-rift-wasm

WebAssembly bindings for tpt-rift. A `Store` is an in-memory encrypted replica.
It does no I/O: JavaScript moves the bytes with `export` and `import`.

- `new Store(node_id, passphrase)` creates an empty store. `Store.load(node_id,
  passphrase, bytes)` restores one from `save()`. A wrong passphrase is an error.
- `save()` returns the whole store, sealed. Persist it, for example in IndexedDB
  (`web/persistence.js`).
- Writes are signed by the store's node. Call `add_peer(node_id, public_key)`
  with a peer's `public_key()` to make `import` refuse changes from unlisted
  nodes or with bad signatures. Trust is not saved, so add peers again after
  `Store.load`.

```sh
wasm-pack build --target nodejs crates/tpt-rift-wasm --out-dir pkg-node
node crates/tpt-rift-wasm/examples/two_stores.cjs
```

Tests run under Node: `wasm-pack test --node crates/tpt-rift-wasm`.

IndexedDB persistence is in `web/` (`persistence.js`), tested under Node with fake-indexeddb.
Not built: a browser demo page.
Design: [ADR 0010](../../docs/adr/0010-wasm-bindings.md).

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
