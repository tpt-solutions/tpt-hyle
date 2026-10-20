//! A replica with a directory applies only events signed by a listed node, and
//! applies nothing from a delta that has one bad event.

use tpt_hyle::{Hyle, Key, VecStorage};
use tpt_rift::{Delta, Directory, Replica, RiftError};

static SESSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn replica(node: u128) -> Replica<VecStorage> {
    let mut session = [0u8; 16];
    let n = SESSIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    session[..8].copy_from_slice(&n.to_le_bytes());
    Replica::new(
        Hyle::open_with_session(
            VecStorage::new(),
            node,
            Key::from_bytes([0x5a; 32]),
            session,
        )
        .unwrap(),
    )
}

/// What `writer` would send to a peer that is not `writer`.
fn export_from(writer: &mut Replica<VecStorage>) -> Delta {
    let cursor = writer.start_cursor();
    writer.export(99, cursor, 1000).unwrap().delta
}

fn directory_with(node: u128, key: [u8; 32]) -> Directory {
    let mut dir = Directory::new();
    dir.insert(node, key).unwrap();
    dir
}

#[test]
fn a_signed_event_from_a_listed_node_is_applied() {
    let mut writer = replica(10);
    writer.store().put(b"k", b"v").unwrap();
    let delta = export_from(&mut writer);

    let mut reader = replica(11);
    reader.set_directory(directory_with(10, writer.public_key()));
    let stats = reader.import(&delta).unwrap();
    assert_eq!(stats.applied, 1);
    assert!(reader.store().contains_key(b"k").unwrap());
}

#[test]
fn a_changed_value_fails_its_signature_and_nothing_is_applied() {
    let mut writer = replica(10);
    writer.store().put(b"k", b"v").unwrap();
    let mut delta = export_from(&mut writer);
    delta.changes[0].value = b"evil".to_vec();

    let mut reader = replica(11);
    reader.set_directory(directory_with(10, writer.public_key()));
    assert!(matches!(
        reader.import(&delta),
        Err(RiftError::Protocol("event signature does not verify"))
    ));
    assert!(!reader.store().contains_key(b"k").unwrap());
}

#[test]
fn a_changed_clock_or_tombstone_flag_fails_too() {
    let mut writer = replica(10);
    writer.store().put(b"k", b"v").unwrap();
    let delta = export_from(&mut writer);

    let mut reader = replica(11);
    reader.set_directory(directory_with(10, writer.public_key()));

    let mut later = delta.clone();
    later.changes[0].lamport += 1;
    assert!(reader.import(&later).is_err(), "lamport is covered");

    let mut deleted = delta.clone();
    deleted.changes[0].tombstone = true;
    deleted.changes[0].value.clear();
    assert!(
        reader.import(&deleted).is_err(),
        "tombstone flag is covered"
    );

    let mut other_node = delta.clone();
    other_node.changes[0].node_id = 12;
    assert!(reader.import(&other_node).is_err(), "node id is covered");
}

#[test]
fn an_unlisted_node_is_refused() {
    let mut writer = replica(10);
    writer.store().put(b"k", b"v").unwrap();
    let delta = export_from(&mut writer);

    let mut reader = replica(11);
    reader.set_directory(directory_with(12, writer.public_key()));
    assert!(matches!(
        reader.import(&delta),
        Err(RiftError::Protocol(
            "change from a node not in the directory"
        ))
    ));
    assert!(!reader.store().contains_key(b"k").unwrap());
}

#[test]
fn one_bad_event_refuses_the_whole_delta() {
    let mut writer = replica(10);
    writer.store().put(b"good", b"1").unwrap();
    writer.store().put(b"bad", b"2").unwrap();
    let mut delta = export_from(&mut writer);
    let bad = delta.changes.iter_mut().find(|c| c.key == b"bad").unwrap();
    bad.value = b"tampered".to_vec();

    let mut reader = replica(11);
    reader.set_directory(directory_with(10, writer.public_key()));
    assert!(reader.import(&delta).is_err());
    assert!(
        !reader.store().contains_key(b"good").unwrap(),
        "the good event in the same delta is not applied either"
    );
}

#[test]
fn without_a_directory_signatures_are_carried_but_not_checked() {
    let mut writer = replica(10);
    writer.store().put(b"k", b"v").unwrap();
    let mut delta = export_from(&mut writer);
    delta.changes[0].signature = [0; 64];

    let mut reader = replica(11);
    assert_eq!(reader.import(&delta).unwrap().applied, 1);
}

#[test]
fn a_node_listed_under_two_keys_is_refused() {
    let mut dir = Directory::new();
    dir.insert(10, [1; 32]).unwrap();
    dir.insert(10, [1; 32]).unwrap();
    assert!(dir.insert(10, [2; 32]).is_err());
    assert_eq!(dir.get(10), Some(&[1; 32]));
    assert_eq!(dir.len(), 1);
}

#[test]
fn an_exported_event_keeps_its_origin_signature() {
    // A relay forwards an event it did not write. The signature still checks
    // against the origin's key, so the relay cannot alter it unnoticed.
    let mut writer = replica(10);
    writer.store().put(b"k", b"v").unwrap();
    let first = export_from(&mut writer);

    let mut relay = replica(11);
    relay.import(&first).unwrap();
    let forwarded = export_from(&mut relay);
    assert_eq!(forwarded.changes[0].signature, first.changes[0].signature);

    let mut third = replica(12);
    third.set_directory(directory_with(10, writer.public_key()));
    assert_eq!(third.import(&forwarded).unwrap().applied, 1);
}
