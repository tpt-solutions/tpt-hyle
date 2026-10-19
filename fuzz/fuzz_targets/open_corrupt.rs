#![no_main]

use libfuzzer_sys::fuzz_target;
use tpt_hyle::{Hyle, Storage, VecStorage, HEADER_LEN};

// Feeds arbitrary bytes in as both segments, then opens and reads. Each
// input must end in Ok or Err, never a panic.
//
// The input is split at its midpoint: the first half becomes the State
// Segment and the second half becomes the Sync Segment.
fuzz_target!(|data: &[u8]| {
    let (state_bytes, sync_bytes) = data.split_at(data.len() / 2);

    let mut state = VecStorage::new();
    let mut sync = VecStorage::new();
    if state.append(state_bytes).is_err() || sync.append(sync_bytes).is_err() {
        return;
    }

    if let Ok(mut db) = Hyle::open(state, sync, 1) {
        let mut buf = [0u8; 256];
        let _ = db.get(b"a", &mut buf);
        let _ = db.get(b"b", &mut buf);
        let _ = db.contains_key(b"c");
        if let Ok(events) = db.events_since(HEADER_LEN) {
            for event in events {
                if let Ok((_, ev)) = event {
                    let mut k = vec![0u8; ev.key_len as usize];
                    let mut v = vec![0u8; ev.payload_len as usize];
                    let _ = db.read_record(&ev, &mut k, &mut v);
                }
            }
        }
    }
});
