use serde_json::json;
use tpt_hyle::{Hyle, Key, VecStorage};
use tpt_rift::{delete_doc, get_doc, put_doc, Replica};

type R = Replica<VecStorage>;

static SESSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A store with a fresh session, so no two opens share nonces.
fn fresh_hyle(node: u128) -> Hyle<VecStorage> {
    let mut session = [0u8; 16];
    let n = SESSIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    session[..8].copy_from_slice(&n.to_le_bytes());
    Hyle::open_with_session(
        VecStorage::new(),
        node,
        Key::from_bytes([0x5a; 32]),
        session,
    )
    .unwrap()
}

fn replica(node: u128) -> R {
    Replica::new(fresh_hyle(node))
}

/// Sends everything each side has to the other, twice, so it settles.
fn sync(a: &mut R, b: &mut R) {
    for _ in 0..2 {
        let from_a = a
            .export(b.node_id(), a.start_cursor(), usize::MAX)
            .unwrap()
            .delta;
        let from_b = b
            .export(a.node_id(), b.start_cursor(), usize::MAX)
            .unwrap()
            .delta;
        b.import(&from_a).unwrap();
        a.import(&from_b).unwrap();
    }
}

#[test]
fn round_trips_awkward_json() {
    let mut r = replica(1);
    let doc = json!({
        "name": "Alice",
        "age": 31,
        "ratio": 0.5,
        "ok": true,
        "nothing": null,
        "tags": ["a", "b/c", "d~e"],
        "nested": {"deep": {"x": 1}, "empty": {}},
        "weird/key~": "escaped",
        "": "empty field name"
    });
    put_doc(r.store(), b"user:1", &doc).unwrap();
    assert_eq!(get_doc(r.store(), b"user:1").unwrap(), Some(doc));
    assert_eq!(get_doc(r.store(), b"absent").unwrap(), None);
}

#[test]
fn root_can_be_a_scalar_or_an_empty_object() {
    let mut r = replica(1);
    put_doc(r.store(), b"s", &json!(42)).unwrap();
    assert_eq!(get_doc(r.store(), b"s").unwrap(), Some(json!(42)));
    put_doc(r.store(), b"e", &json!({})).unwrap();
    assert_eq!(get_doc(r.store(), b"e").unwrap(), Some(json!({})));
}

#[test]
fn rewriting_a_document_removes_fields_that_disappeared() {
    let mut r = replica(1);
    put_doc(r.store(), b"d", &json!({"a": 1, "b": {"c": 2}})).unwrap();
    put_doc(r.store(), b"d", &json!({"a": 1})).unwrap();
    assert_eq!(get_doc(r.store(), b"d").unwrap(), Some(json!({"a": 1})));
}

#[test]
fn delete_removes_the_whole_document() {
    let mut r = replica(1);
    put_doc(r.store(), b"d", &json!({"a": 1})).unwrap();
    delete_doc(r.store(), b"d").unwrap();
    assert_eq!(get_doc(r.store(), b"d").unwrap(), None);
}

#[test]
fn documents_with_prefix_related_ids_do_not_mix() {
    let mut r = replica(1);
    put_doc(r.store(), b"a", &json!({"x": 1})).unwrap();
    put_doc(r.store(), b"ab", &json!({"y": 2})).unwrap();
    assert_eq!(get_doc(r.store(), b"a").unwrap(), Some(json!({"x": 1})));
    assert_eq!(get_doc(r.store(), b"ab").unwrap(), Some(json!({"y": 2})));
}

#[test]
fn concurrent_edits_to_different_fields_merge() {
    let mut a = replica(1);
    let mut b = replica(2);
    put_doc(a.store(), b"u", &json!({"name": "Alice", "city": "Oslo"})).unwrap();
    sync(&mut a, &mut b);
    // Both edit while apart. They touch different fields.
    put_doc(a.store(), b"u", &json!({"name": "Alicia", "city": "Oslo"})).unwrap();
    put_doc(b.store(), b"u", &json!({"name": "Alice", "city": "Bergen"})).unwrap();
    sync(&mut a, &mut b);

    let merged = json!({"name": "Alicia", "city": "Bergen"});
    assert_eq!(get_doc(a.store(), b"u").unwrap(), Some(merged.clone()));
    assert_eq!(get_doc(b.store(), b"u").unwrap(), Some(merged));
}

#[test]
fn concurrent_edits_to_the_same_field_resolve_by_lww() {
    let mut a = replica(1);
    let mut b = replica(2);
    put_doc(a.store(), b"u", &json!({"name": "start"})).unwrap();
    sync(&mut a, &mut b);
    // Both lamports are equal here, so node id breaks the tie: node 2 wins.
    put_doc(a.store(), b"u", &json!({"name": "from-a"})).unwrap();
    put_doc(b.store(), b"u", &json!({"name": "from-b"})).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(
        get_doc(a.store(), b"u").unwrap(),
        Some(json!({"name": "from-b"}))
    );
    assert_eq!(
        get_doc(b.store(), b"u").unwrap(),
        Some(json!({"name": "from-b"}))
    );
}

#[test]
fn ancestor_and_descendant_conflicts_are_deterministic() {
    // One replica writes a scalar at /x. The other writes an object at /x.
    let mut a = replica(1);
    let mut b = replica(2);
    put_doc(a.store(), b"d", &json!({"x": 5})).unwrap();
    put_doc(b.store(), b"d", &json!({"x": {"y": 1}})).unwrap();
    sync(&mut a, &mut b);
    let da = get_doc(a.store(), b"d").unwrap();
    let db = get_doc(b.store(), b"d").unwrap();
    assert_eq!(da, db, "both replicas must agree");
    assert_eq!(da, Some(json!({"x": {"y": 1}})), "the descendant wins");
}
