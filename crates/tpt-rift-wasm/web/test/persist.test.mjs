// Run with `npm test` in crates/tpt-rift-wasm/web, after building the Node
// package with: wasm-pack build --target nodejs crates/tpt-rift-wasm --out-dir pkg-node

import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import "fake-indexeddb/auto";
import { Persistence } from "../persistence.js";

const require = createRequire(import.meta.url);
const { Store } = require("../../pkg-node/tpt_rift_wasm.js");

const bytes = (s) => new TextEncoder().encode(s);
const text = (b) => (b === undefined ? undefined : new TextDecoder().decode(b));

let counter = 0;
const freshName = () => `test-db-${process.pid}-${counter++}`;

test("a saved store is restored with its data and clock", async () => {
  const name = freshName();
  const first = await new Persistence(indexedDB, name).open();
  const store = new Store("1");
  store.put(bytes("note"), bytes("saved before a reload"));
  store.delete(bytes("never-there"));
  await first.save(store);
  first.close();

  // A new Persistence on the same database stands in for a page reload.
  const second = await new Persistence(indexedDB, name).open();
  const restored = await second.load("1", Store);
  assert.equal(text(restored.get(bytes("note"))), "saved before a reload");
  assert.equal(restored.lamport(), store.lamport());
  second.close();
});

test("loading when nothing was saved returns null", async () => {
  const p = await new Persistence(indexedDB, freshName()).open();
  assert.equal(await p.load("1", Store), null);
  p.close();
});

test("a later save replaces the earlier snapshot", async () => {
  const name = freshName();
  const p = await new Persistence(indexedDB, name).open();
  const store = new Store("1");
  store.put(bytes("k"), bytes("v1"));
  await p.save(store);
  store.put(bytes("k"), bytes("v2"));
  await p.save(store);
  const restored = await p.load("1", Store);
  assert.equal(text(restored.get(bytes("k"))), "v2");
  p.close();
});

test("a torn tail in the saved sync segment is recovered, not trusted", async () => {
  const name = freshName();
  const p = await new Persistence(indexedDB, name).open();
  const store = new Store("1");
  store.put(bytes("a"), bytes("committed"));
  store.put(bytes("b"), bytes("also committed"));
  // Simulate a save that was cut short: the sync segment ends mid-record.
  const sync = store.sync_bytes();
  const torn = new Uint8Array([...sync.slice(0, sync.length - 20)]);
  const state = store.state_bytes();
  const record = { state, sync: torn };
  const tx = p.db.transaction("segments", "readwrite");
  tx.objectStore("segments").put(record, "current");
  await new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onerror = () => reject(tx.error);
  });

  const restored = await p.load("1", Store);
  assert.equal(text(restored.get(bytes("a"))), "committed");
  assert.equal(restored.get(bytes("b")), undefined, "the torn write is dropped");
  p.close();
});

test("the node id of a saved store must match when it is loaded", async () => {
  const name = freshName();
  const p = await new Persistence(indexedDB, name).open();
  const store = new Store("1");
  store.put(bytes("k"), bytes("v"));
  await p.save(store);
  await assert.rejects(p.load("2", Store));
  p.close();
});
