//! Two file-backed replicas that write offline, then sync over an in-process
//! transport.
//!
//! ```text
//! cargo run -p tpt-hyle-rift --example two_replicas -- <scratch-dir>
//! ```

use tpt_hyle_rift::{connect, open};
use tpt_rift::MockTransport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "two-replicas".into()),
    );
    std::fs::create_dir_all(&dir)?;
    for name in ["laptop", "phone"] {
        for ext in ["sync", "sync.tmp"] {
            let _ = std::fs::remove_file(dir.join(format!("{name}.{ext}")));
        }
    }

    let mut laptop = open(dir.join("laptop"), 1)?;
    let mut phone = open(dir.join("phone"), 2)?;

    // Both sides write while offline.
    laptop.store().put(b"note:1", b"written on the laptop")?;
    phone.store().put(b"note:2", b"written on the phone")?;
    phone
        .store()
        .put(b"note:1", b"edited on the phone, later")?;

    // Connect and sync.
    let (laptop_end, phone_end) = MockTransport::pair();
    let mut laptop_sync = connect(&mut laptop, 2, laptop_end);
    let mut phone_sync = connect(&mut phone, 1, phone_end);
    for _ in 0..2 {
        laptop_sync.resume().await?;
        phone_sync.resume().await?;
    }

    for (name, db) in [("laptop", &mut laptop), ("phone", &mut phone)] {
        for key in [&b"note:1"[..], &b"note:2"[..]] {
            let mut buf = [0u8; 128];
            let n = db.store().get(key, &mut buf)?.unwrap_or(0);
            println!(
                "{name}: {} = {}",
                String::from_utf8_lossy(key),
                String::from_utf8_lossy(&buf[..n])
            );
        }
    }
    Ok(())
}
