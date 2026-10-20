use tpt_rift_uniffi::{SyncError, SyncStore};

const PASS: &str = "test passphrase";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tpt-rift-uniffi-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn two_devices_sync_through_the_sdk_surface() {
    let dir = scratch("sync");
    let phone = SyncStore::open(
        dir.join("phone").display().to_string(),
        "1".into(),
        PASS.into(),
    )
    .unwrap();
    let laptop = SyncStore::open(
        dir.join("laptop").display().to_string(),
        "2".into(),
        PASS.into(),
    )
    .unwrap();

    phone
        .put(b"note".to_vec(), b"from the phone".to_vec())
        .unwrap();
    laptop
        .put(b"todo".to_vec(), b"from the laptop".to_vec())
        .unwrap();

    let to_laptop = phone.export_for("2".into()).unwrap();
    let to_phone = laptop.export_for("1".into()).unwrap();
    assert_eq!(laptop.import_from(to_laptop).unwrap(), 1);
    assert_eq!(phone.import_from(to_phone).unwrap(), 1);

    assert_eq!(
        laptop.get(b"note".to_vec()).unwrap().as_deref(),
        Some(&b"from the phone"[..])
    );
    assert_eq!(
        phone.get(b"todo".to_vec()).unwrap().as_deref(),
        Some(&b"from the laptop"[..])
    );
}

#[test]
fn data_survives_reopening_the_store() {
    let dir = scratch("reopen");
    let path = dir.join("db").display().to_string();
    {
        let s = SyncStore::open(path.clone(), "7".into(), PASS.into()).unwrap();
        s.put(b"k".to_vec(), b"kept".to_vec()).unwrap();
        assert!(s.delete(b"gone".to_vec()).is_ok());
    }
    let s = SyncStore::open(path, "7".into(), PASS.into()).unwrap();
    assert_eq!(s.get(b"k".to_vec()).unwrap().as_deref(), Some(&b"kept"[..]));
    assert_eq!(s.lamport().unwrap(), 1);
}

#[test]
fn malformed_input_is_an_error_not_a_panic() {
    let dir = scratch("bad");
    let path = dir.join("db").display().to_string();
    assert!(matches!(
        SyncStore::open(path.clone(), "not a number".into(), PASS.into()),
        Err(SyncError::Invalid { .. })
    ));
    let s = SyncStore::open(path, "3".into(), PASS.into()).unwrap();
    assert!(matches!(
        s.import_from(b"junk".to_vec()),
        Err(SyncError::Invalid { .. })
    ));
    assert!(matches!(
        s.export_for("x".into()),
        Err(SyncError::Invalid { .. })
    ));
}
