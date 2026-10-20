//! Writes a few keys to a file-backed store, then reopens it.
//!
//! ```text
//! cargo run -p tpt-hyle --example basic -- path/to/demo
//! cargo run -p tpt-hyle-cli -- dump-sync-log path/to/demo
//! ```

use tpt_hyle::Hyle;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::args().nth(1).unwrap_or_else(|| "demo".into());
    let node_id = 0x0000_0000_0000_0000_0000_0000_0000_0001;

    {
        let mut db = Hyle::open_file(&base, node_id, b"example passphrase")?;
        db.put(b"user:101", b"alice")?;
        db.put(b"user:102", b"bob")?;
        db.put(b"user:101", b"alice-updated")?;
        db.delete(b"user:102")?;
    }

    let mut db = Hyle::open_file(&base, node_id, b"example passphrase")?;
    let mut buf = [0u8; 64];
    match db.get(b"user:101", &mut buf)? {
        Some(n) => println!("user:101 = {}", String::from_utf8_lossy(&buf[..n])),
        None => println!("user:101 is missing"),
    }
    println!("user:102 present: {}", db.contains_key(b"user:102")?);
    println!("lamport: {}", db.lamport());
    Ok(())
}
