use tpt_rift_wasm::Store;
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
fn two_stores_sync_by_passing_bytes() {
    let mut a = Store::new("1").unwrap();
    let mut b = Store::new("2").unwrap();
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
    let mut a = Store::new("1").unwrap();
    let mut b = Store::new("2").unwrap();
    a.put(b"k", b"v").unwrap();
    let bytes = a.export("2").unwrap();
    assert_eq!(b.import(&bytes).unwrap(), 1);
    assert_eq!(b.import(&bytes).unwrap(), 0, "the second copy wins nothing");
    assert_eq!(b.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
}

#[wasm_bindgen_test]
fn deletes_propagate() {
    let mut a = Store::new("1").unwrap();
    let mut b = Store::new("2").unwrap();
    a.put(b"k", b"v").unwrap();
    b.import(&a.export("2").unwrap()).unwrap();
    assert!(a.delete(b"k").unwrap());
    b.import(&a.export("2").unwrap()).unwrap();
    assert_eq!(b.get(b"k").unwrap(), None);
}

#[wasm_bindgen_test]
fn malformed_bytes_are_an_error_not_a_trap() {
    let mut a = Store::new("1").unwrap();
    assert!(a.import(b"not a delta").is_err());
    assert!(Store::new("not a number").is_err());
}
