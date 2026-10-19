#![cfg(feature = "json")]

use serde_json::json;
use tpt_hyle::{Hyle, VecStorage};

#[test]
fn json_round_trips_through_the_store() {
    let mut db = Hyle::open(VecStorage::new(), VecStorage::new(), 1).unwrap();
    let doc = json!({"name": "Alice", "tags": [1, 2, 3], "ok": true});
    db.put_json(b"user:101", &doc).unwrap();

    let back: Option<serde_json::Value> = db.get_json(b"user:101").unwrap();
    assert_eq!(back, Some(doc));
}

#[test]
fn json_absent_and_deleted_keys_read_as_none() {
    let mut db = Hyle::open(VecStorage::new(), VecStorage::new(), 1).unwrap();
    assert_eq!(db.get_json::<serde_json::Value>(b"nope").unwrap(), None);
    db.put_json(b"k", &json!(1)).unwrap();
    db.delete(b"k").unwrap();
    assert_eq!(db.get_json::<serde_json::Value>(b"k").unwrap(), None);
}

#[test]
fn json_type_mismatch_is_an_error_not_a_panic() {
    let mut db = Hyle::open(VecStorage::new(), VecStorage::new(), 1).unwrap();
    db.put(b"k", b"not json at all").unwrap();
    assert!(db.get_json::<serde_json::Value>(b"k").is_err());
}
