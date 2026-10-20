#![no_main]

use libfuzzer_sys::fuzz_target;
use tpt_hyle::{Hyle, Key, Storage, VecStorage, HEADER_LEN};

const KEY: [u8; 32] = [0x11; 32];

// Two paths, each of which must end in Ok or Err, never a panic or an
// unbounded allocation.
//
// 1. The whole input is a store file. Almost nothing authenticates, so this
//    mostly exercises the header and frame-length checks.
// 2. A valid store is written, then one byte of its file is flipped at an
//    offset the input chooses. This reaches recovery and the commit checks.
fuzz_target!(|data: &[u8]| {
    let mut storage = VecStorage::new();
    if storage.append(data).is_ok() {
        if let Ok(db) = Hyle::open_with_session(storage, 1, Key::from_bytes(KEY), [1; 16]) {
            read_everything(db);
        }
    }

    let mut db = match Hyle::open_with_session(VecStorage::new(), 1, Key::from_bytes(KEY), [2; 16]) {
        Ok(db) => db,
        Err(_) => return,
    };
    if db.put(b"a", b"alpha").is_err() || db.put(b"b", b"beta").is_err() {
        return;
    }
    let mut bytes = db.into_storage().bytes().to_vec();
    if let Some((&index, &flip)) = data.first().zip(data.get(1)) {
        let at = (index as usize * 256 + index as usize) % bytes.len();
        bytes[at] ^= flip | 1;
    }
    let mut storage = VecStorage::new();
    if storage.append(&bytes).is_ok() {
        if let Ok(db) = Hyle::open_with_session(storage, 1, Key::from_bytes(KEY), [3; 16]) {
            read_everything(db);
        }
    }
});

fn read_everything(mut db: Hyle<VecStorage>) {
    let mut buf = [0u8; 256];
    let _ = db.get(b"a", &mut buf);
    let _ = db.contains_key(b"c");
    let Ok(events) = db.events_since(HEADER_LEN) else {
        return;
    };
    let events: Vec<_> = events.flatten().map(|(_, ev)| ev).collect();
    for ev in events {
        let mut k = vec![0u8; ev.key_len as usize];
        let mut v = vec![0u8; ev.payload_len as usize];
        let _ = db.read_record(&ev, &mut k, &mut v);
    }
}
