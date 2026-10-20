# ADR 0007: JSON document CRDT

Status: accepted

## Context

The TODO's Phase 2 item is a "basic JSON CRDT with field-level LWW". A
document has many fields, and concurrent edits to different fields should
merge without losing either. Concurrent edits to the same field should resolve
to one value, on every replica.

## Decision

Each leaf of a document is one key in the store. A leaf is any JSON value
that is not a non-empty object: scalars, arrays, `null`, and empty objects.
Keys have the form:

    [doc_id length: u16 BE] [doc_id bytes] [RFC 6901 pointer]

The length prefix keeps documents from colliding, because a document id can
never be the prefix of another document's keys. Pointer segments are escaped
as RFC 6901 specifies (`~` becomes `~0`, `/` becomes `~1`).

Writing a document is one batch. It writes only the leaves whose bytes
changed and deletes the leaves that disappeared. Reading a document scans the
live entries under its prefix and rebuilds the value.

## Why per-leaf keys

- **Concurrent edits to different fields merge.** Each field is an independent
  LWW register, so the store's existing rule resolves each field separately.
- **No new replication code.** Documents travel as ordinary changes. The
  convergence and origin rules from ADR 0004 apply unchanged.
- **Simple deletion.** Removing a field is a tombstone on one key.

## Conflicts between an object and a scalar

One replica writes a scalar at `/x`. Another writes an object at `/x`, with
fields under it. Both leaves are live. The rule is that the **descendant
wins**, and the scalar is replaced by the object. Leaves are applied in
pointer-length order, so an ancestor is always placed before its
descendants. Every replica that holds the same leaves produces the same
document.

## Limits

- Arrays are leaves. Two replicas that edit different elements of one array
  will conflict on the whole array, and the later write wins. An element-level
  array CRDT is not built.
- The whole log is scanned on each read or write of a document. This is
  linear in log length, the same as `get` today, and it gets the same fix
  when a key index exists.
- Document ids are at most 65535 bytes.
- The root may be a scalar or an empty object. A non-empty root object is
  flattened like any other.

## Consequences

- Each field is one store record, so a document with many fields costs one
  commit record per field. Writes are batched, so this costs state and data
  records, not extra flushes.
- The `json` feature of `tpt-hyle` (serde_json in the store) is separate from
  this CRDT. This one lives in `tpt-rift` and uses serde_json directly.
