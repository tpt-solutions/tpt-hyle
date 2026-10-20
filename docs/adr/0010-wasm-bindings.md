# ADR 0010: WebAssembly bindings without IndexedDB

> **Updated.** The store is encrypted now. `Store::new` and `Store::load` take a passphrase, and `save` returns sealed bytes. See [ADR 0013](0013-encryption-and-single-file.md).


Status: accepted

## Context

The Phase 4 TODO asks for WebAssembly bindings with IndexedDB as the browser
store. The storage trait is synchronous. IndexedDB is asynchronous, and it
cannot be read or written from inside a synchronous call.

## Decision

`tpt-rift-wasm` exposes an in-memory `Store`. It owns a `VecStorage` replica
and performs no I/O. JavaScript moves the bytes:

- `export(peer)` returns the encoded delta for a peer, which JavaScript sends.
- `import(bytes)` applies a delta that JavaScript received.

Import is idempotent, so a transport that duplicates or reorders messages is
fine. The bindings contain no networking and no runtime.

## Why not IndexedDB now

Persistence needs an asynchronous storage path: JavaScript writes the segments
after each commit and reads them back at startup. The clean design is to put
that in the JavaScript layer, outside the synchronous core, with a commit
callback. That design is not built. Until it is, data lasts as long as the
page.

## Verification

- Four `wasm-bindgen-test` tests run under Node: two-store exchange, duplicate
  imports, deletes, and malformed input.
- `crates/tpt-rift-wasm/examples/two_stores.cjs` runs the same flow from a
  Node script against the `wasm-pack` output.
- Not tested in a real browser.

## Consequences

- The wasm build is small and runs anywhere wasm does.
- Nothing survives a page reload yet.
- A browser demo page is not written. The Node example shows the same calls.

## Update: IndexedDB persistence

Persistence is now built, in the JavaScript layer, as the decision above
prescribes. `web/persistence.js` writes both segments in one IndexedDB
transaction after each save, and `Store.load` restores them. Recovery runs on
load, so a save cut short by a crash leaves a tail that is dropped, as it would
be on disk.

Each save writes the whole store. That is O(size) per save. Incremental saves
are the next step, and the core already supports them, since the segments are
append-only between compactions.

Tested under Node with `fake-indexeddb`, which is a real IndexedDB
implementation. The tests cover restore, a missing database, replacement by a
later save, a torn tail, and a node id mismatch. Not tested in a browser.
