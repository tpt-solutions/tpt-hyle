// Two in-memory stores that sync by passing bytes, as two browser tabs would.
//
// Build the package first:
//   wasm-pack build --target nodejs crates/tpt-rift-wasm --out-dir pkg-node
// Then run:
//   node crates/tpt-rift-wasm/examples/two_stores.cjs

const { Store } = require("../pkg-node/tpt_rift_wasm.js");

const text = (bytes) => (bytes === undefined ? "(missing)" : Buffer.from(bytes).toString());

const phone = new Store("1");
const laptop = new Store("2");

// Both write while "offline".
phone.put(Buffer.from("note:1"), Buffer.from("written on the phone"));
laptop.put(Buffer.from("note:2"), Buffer.from("written on the laptop"));
laptop.put(Buffer.from("note:1"), Buffer.from("edited on the laptop, later"));

// Reconnect: each side exports what the other lacks, and imports what arrives.
// Sending the same bytes twice is harmless.
const toLaptop = phone.export("2");
const toPhone = laptop.export("1");
laptop.import(toLaptop);
laptop.import(toLaptop);
phone.import(toPhone);

for (const [name, store] of [["phone", phone], ["laptop", laptop]]) {
  for (const key of ["note:1", "note:2"]) {
    console.log(`${name}: ${key} = ${text(store.get(Buffer.from(key)))}`);
  }
}
