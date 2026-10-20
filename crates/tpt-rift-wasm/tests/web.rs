use tpt_rift_wasm::Store;
use wasm_bindgen_test::*;

const PASS: &[u8] = b"test passphrase";

#[wasm_bindgen_test]
fn two_stores_sync_by_passing_bytes() {
    let mut a = Store::new("1", PASS).unwrap();
    let mut b = Store::new("2", PASS).unwrap();
    a.put(b"note:1", b"from a").unwrap();
    b.put(b"note:2", b"from b").unwrap();

    let to_b = a.export("2").unwrap();
    let to_a = b.export("1").unwrap();
    assert_eq!(b.import(&to_b).unwrap(), 1);
    assert_eq!(a.import(&to_a).unwrap(), 1);

    assert_eq!(a.get(b"note:2").unwrap().as_deref(), Some(&b"from b"[..]));
    assert_eq!(b.get(b"note:1").unwrap().as_deref(), Some(&b"from a"[..]));
}

#[wasm_bindgen_test]
fn duplicate_imports_are_harmless() {
    let mut a = Store::new("1", PASS).unwrap();
    let mut b = Store::new("2", PASS).unwrap();
    a.put(b"k", b"v").unwrap();
    let bytes = a.export("2").unwrap();
    assert_eq!(b.import(&bytes).unwrap(), 1);
    assert_eq!(b.import(&bytes).unwrap(), 0, "the second copy wins nothing");
    assert_eq!(b.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
}

#[wasm_bindgen_test]
fn deletes_propagate() {
    let mut a = Store::new("1", PASS).unwrap();
    let mut b = Store::new("2", PASS).unwrap();
    a.put(b"k", b"v").unwrap();
    b.import(&a.export("2").unwrap()).unwrap();
    assert!(a.delete(b"k").unwrap());
    b.import(&a.export("2").unwrap()).unwrap();
    assert_eq!(b.get(b"k").unwrap(), None);
}

#[wasm_bindgen_test]
fn saved_bytes_restore_the_store_with_the_passphrase() {
    let mut a = Store::new("1", PASS).unwrap();
    a.put(b"k", b"persisted").unwrap();
    let bytes = a.save();
    let mut restored = Store::load("1", PASS, &bytes).unwrap();
    assert_eq!(
        restored.get(b"k").unwrap().as_deref(),
        Some(&b"persisted"[..])
    );
}

#[wasm_bindgen_test]
fn saved_bytes_do_not_open_with_the_wrong_passphrase() {
    let mut a = Store::new("1", PASS).unwrap();
    a.put(b"k", b"secret").unwrap();
    let bytes = a.save();
    assert!(Store::load("1", b"wrong", &bytes).is_err());
}

#[wasm_bindgen_test]
fn malformed_input_is_an_error_not_a_trap() {
    let mut a = Store::new("1", PASS).unwrap();
    assert!(a.import(b"not a delta").is_err());
    assert!(Store::new("not a number", PASS).is_err());
}

#[wasm_bindgen_test]
fn a_trusted_peer_imports_and_a_forged_change_does_not() {
    let mut a = Store::new("1", PASS).unwrap();
    let mut b = Store::new("2", PASS).unwrap();
    a.put(b"k", b"v").unwrap();
    b.add_peer("1", &a.public_key()).unwrap();

    let mut bytes = a.export("2").unwrap();
    let at = bytes.len() - 65;
    bytes[at] ^= 1;
    assert!(b.import(&bytes).is_err());
    assert_eq!(b.get(b"k").unwrap(), None);

    let good = a.export("2").unwrap();
    assert_eq!(b.import(&good).unwrap(), 1);
    assert_eq!(b.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
}

#[wasm_bindgen_test]
fn a_bad_public_key_is_refused() {
    let mut a = Store::new("1", PASS).unwrap();
    assert!(a.add_peer("2", "not hex").is_err());
    assert_eq!(a.public_key().len(), 64);
}
