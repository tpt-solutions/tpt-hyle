//! A JSON document CRDT with field-level last-writer-wins.
//!
//! A document is stored as one key per leaf. A leaf is any JSON value that is
//! not a non-empty object: scalars, arrays, and empty objects. Each key is
//! `[doc_id_len: u16 BE] [doc_id] [RFC 6901 pointer]`. Two replicas that edit
//! different fields therefore merge without conflict. Two that edit the same
//! field resolve by the usual last-writer-wins on the store.
//!
//! When a leaf is an ancestor of another leaf, the descendant wins. The
//! ancestor leaf is replaced by an object. Both replicas apply the same rule
//! to the same set of leaves, so they still agree.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use tpt_hyle::{Hyle, Storage};

use crate::RiftError;

/// Key prefix for every leaf of document `doc`.
fn doc_prefix(doc: &[u8]) -> Result<Vec<u8>, RiftError> {
    let len = u16::try_from(doc.len())
        .map_err(|_| RiftError::Protocol("document id longer than 65535 bytes"))?;
    let mut prefix = len.to_be_bytes().to_vec();
    prefix.extend_from_slice(doc);
    Ok(prefix)
}

/// RFC 6901 escaping for one pointer segment.
fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

/// Appends one leaf per non-empty-object node of `value`. `base` is the key
/// of `value` itself.
fn flatten(value: &Value, base: Vec<u8>, out: &mut BTreeMap<Vec<u8>, Vec<u8>>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (field, child) in map {
                let mut key = base.clone();
                key.push(b'/');
                key.extend_from_slice(escape(field).as_bytes());
                flatten(child, key, out);
            }
        }
        leaf => {
            // Serializing a `Value` cannot fail.
            let bytes = serde_json::to_vec(leaf).expect("a JSON value always serializes");
            out.insert(base, bytes);
        }
    }
}

/// Sets `leaf` at `segments` under `root`, creating objects as needed. A
/// non-object in the way is replaced by an object, so a descendant wins over
/// an ancestor leaf.
fn insert(root: &mut Value, segments: &[String], leaf: Value) {
    let Some((first, rest)) = segments.split_first() else {
        *root = leaf;
        return;
    };
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    if let Value::Object(map) = root {
        insert(map.entry(first.clone()).or_insert(Value::Null), rest, leaf);
    }
}

/// Rebuilds a document from its leaves, given as `(pointer, value)` pairs.
fn assemble(mut leaves: Vec<(String, Value)>) -> Option<Value> {
    if leaves.is_empty() {
        return None;
    }
    // Ancestors have strictly shorter pointers, so this order places each
    // ancestor before its descendants. That is what makes descendants win.
    leaves.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
    let mut root = Value::Null;
    for (pointer, leaf) in leaves {
        let segments: Vec<String> = if pointer.is_empty() {
            Vec::new()
        } else {
            pointer.split('/').skip(1).map(unescape).collect()
        };
        insert(&mut root, &segments, leaf);
    }
    Some(root)
}

/// Stores `value` as document `doc`. Only fields that changed are written,
/// all in one batch.
pub fn put_doc<St: Storage, Sy: Storage>(
    store: &mut Hyle<St, Sy>,
    doc: &[u8],
    value: &Value,
) -> Result<(), RiftError> {
    let prefix = doc_prefix(doc)?;
    let mut desired = BTreeMap::new();
    flatten(value, prefix.clone(), &mut desired);
    let current: BTreeMap<Vec<u8>, Vec<u8>> = store.live_entries(&prefix)?.into_iter().collect();

    let mut batch = store.batch()?;
    for (key, bytes) in &desired {
        if current.get(key) != Some(bytes) {
            batch.put(key, bytes)?;
        }
    }
    for key in current.keys() {
        if !desired.contains_key(key) {
            batch.delete(key)?;
        }
    }
    batch.commit()?;
    Ok(())
}

/// Reads document `doc`, or `None` if no field of it is live.
pub fn get_doc<St: Storage, Sy: Storage>(
    store: &mut Hyle<St, Sy>,
    doc: &[u8],
) -> Result<Option<Value>, RiftError> {
    let prefix = doc_prefix(doc)?;
    let mut leaves = Vec::new();
    for (key, bytes) in store.live_entries(&prefix)? {
        let pointer = String::from_utf8(key[prefix.len()..].to_vec())
            .map_err(|_| RiftError::Protocol("document pointer is not UTF-8"))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|e| RiftError::Json(e.to_string()))?;
        leaves.push((pointer, value));
    }
    Ok(assemble(leaves))
}

/// Deletes every field of document `doc`.
pub fn delete_doc<St: Storage, Sy: Storage>(
    store: &mut Hyle<St, Sy>,
    doc: &[u8],
) -> Result<(), RiftError> {
    let prefix = doc_prefix(doc)?;
    let keys: Vec<Vec<u8>> = store
        .live_entries(&prefix)?
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let mut batch = store.batch()?;
    for key in &keys {
        batch.delete(key)?;
    }
    batch.commit()?;
    Ok(())
}
