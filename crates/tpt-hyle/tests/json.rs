#![cfg(feature = "json")]

use serde_json::json;
use tpt_hyle::{Hyle, Key, VecStorage};

fn fresh() -> Hyle<VecStorage> {
    Hyle::open_with_session(VecStorage::new(), 1, Key::from_bytes([9; 32]), [4; 16]).unwrap()
}

#[test]
fn json_round_trips_through_the_store() {
    let mut db = fresh();
    let doc = json!({"name": "Alice", "age": 31, "tags": ["a", "b"], "ok": true});
    db.put_json(b"user:1", &doc).unwrap();
    let back: Option<serde_json::Value> = db.get_json(b"user:1").unwrap();
    assert_eq!(back, Some(doc));
}

#[test]
fn json_absent_and_deleted_keys_read_as_none() {
    let mut db = fresh();
    assert_eq!(db.get_json::<serde_json::Value>(b"nope").unwrap(), None);
    db.put_json(b"k", &json!(1)).unwrap();
    db.delete(b"k").unwrap();
    assert_eq!(db.get_json::<serde_json::Value>(b"k").unwrap(), None);
}

#[test]
fn json_type_mismatch_is_an_error_not_a_panic() {
    let mut db = fresh();
    db.put(b"k", b"not json at all").unwrap();
    assert!(db.get_json::<serde_json::Value>(b"k").is_err());
}
