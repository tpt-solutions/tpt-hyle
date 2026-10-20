// Saves a tpt-rift wasm store to IndexedDB, and restores it on the next load.
//
// The store is synchronous and in memory. This layer owns persistence. After
// each change the caller calls `save`, which writes the store's encrypted bytes
// in one IndexedDB transaction. A transaction lands whole or not at all, so a
// save interrupted by a crash leaves the previous snapshot in place.
//
// Each save writes the whole store. That is O(size) per save. The bytes are
// encrypted, so the database holds nothing readable without the passphrase.

const STORE_NAME = "segments";
const KEY = "current";

function request(req) {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function transactionDone(tx) {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error ?? new Error("transaction aborted"));
  });
}

export class Persistence {
  /**
   * @param {IDBFactory} idb  The IndexedDB factory: `indexedDB` in a browser.
   * @param {string} name     The database name. Use one per store.
   */
  constructor(idb, name = "tpt-rift") {
    this.idb = idb;
    this.name = name;
    this.db = null;
  }

  /** Opens the database, creating it on first use. Call once before save or load. */
  async open() {
    const req = this.idb.open(this.name, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME);
      }
    };
    this.db = await request(req);
    return this;
  }

  /** Writes the store's encrypted bytes in one transaction. */
  async save(store) {
    const tx = this.db.transaction(STORE_NAME, "readwrite");
    tx.objectStore(STORE_NAME).put({ bytes: store.save() }, KEY);
    await transactionDone(tx);
  }

  /**
   * Restores the saved store for `nodeId` with `passphrase`, using
   * `StoreClass.load`. Returns `null` if nothing has been saved yet. Throws if
   * the passphrase does not match.
   */
  async load(nodeId, passphrase, StoreClass) {
    const tx = this.db.transaction(STORE_NAME, "readonly");
    const record = await request(tx.objectStore(STORE_NAME).get(KEY));
    if (!record) {
      return null;
    }
    return StoreClass.load(nodeId, passphrase, record.bytes);
  }

  /** Closes the database. */
  close() {
    this.db?.close();
    this.db = null;
  }
}
