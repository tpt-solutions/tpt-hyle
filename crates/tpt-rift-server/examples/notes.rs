//! A small notes client for the demo. It keeps notes in a local file-backed
//! store, and syncs them with a running `tpt-rift-server`.
//!
//! ```text
//! TPT_CLIENT_TOKEN=... TPT_CLIENT_PASSPHRASE=... \
//!   cargo run -p tpt-rift-server --example notes -- <db> <ws-url> <node-id> <hub-node-id> put <key> <text>
//! TPT_CLIENT_TOKEN=... TPT_CLIENT_PASSPHRASE=... \
//!   cargo run -p tpt-rift-server --example notes -- <db> <ws-url> <node-id> <hub-node-id> get <key>
//! TPT_CLIENT_TOKEN=... TPT_CLIENT_PASSPHRASE=... \
//!   cargo run -p tpt-rift-server --example notes -- <db> <ws-url> <node-id> <hub-node-id> sync
//! ```
//!
//! `put` and `get` work offline. `sync` connects to the hub, pushes local
//! writes, and pulls the other clients' writes. The token and passphrase come
//! from the environment, never from the command line.

use tpt_rift_transport_ws::connect_authorized;

const VALUE_BUF: usize = 64 * 1024;
const SYNC_ROUNDS: usize = 5;
const ROUND_PAUSE: std::time::Duration = std::time::Duration::from_millis(150);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 5 {
        return Err(
            "usage: notes <db> <ws-url> <node-id> <hub-node-id> put|get|sync [key] [text]".into(),
        );
    }
    let (db_path, url, node, hub_node) = (&args[0], &args[1], &args[2], &args[3]);
    let node: u128 = node.parse()?;
    let hub_node: u128 = hub_node.parse()?;
    let passphrase = std::env::var("TPT_CLIENT_PASSPHRASE")
        .map_err(|_| "set TPT_CLIENT_PASSPHRASE to the notes store passphrase")?;
    let mut db = tpt_hyle_rift::open(db_path, node, passphrase.as_bytes())?;

    match args[4].as_str() {
        "put" => {
            let (key, text) = match (args.get(5), args.get(6)) {
                (Some(k), Some(t)) => (k, t),
                _ => {
                    return Err(
                        "usage: notes <db> <ws-url> <node-id> <hub-node-id> put <key> <text>"
                            .into(),
                    )
                }
            };
            db.store().put(key.as_bytes(), text.as_bytes())?;
            println!("saved {key} locally; run `sync` to send it");
        }
        "get" => {
            let key = args.get(5).ok_or("usage: ... get <key>")?;
            let mut buf = vec![0u8; VALUE_BUF];
            match db.store().get(key.as_bytes(), &mut buf)? {
                Some(n) => println!("{}", String::from_utf8_lossy(&buf[..n])),
                None => println!("(no note at {key})"),
            }
        }
        "sync" => {
            let token = std::env::var("TPT_CLIENT_TOKEN")
                .map_err(|_| "set TPT_CLIENT_TOKEN to this client's token")?;
            let transport = connect_authorized(url, &token).await?;
            let mut session = tpt_hyle_rift::connect(&mut db, hub_node, transport);
            // The hub answers on its own poll interval, so a single round can
            // return before its reply arrives. Several paced rounds let the
            // reply land: the first sends local writes, the rest pick up the
            // hub's log.
            for _ in 0..SYNC_ROUNDS {
                let stats = session.resume().await?;
                println!("round done: {stats:?}");
                tokio::time::sleep(ROUND_PAUSE).await;
            }
            session.close().await?;
        }
        other => return Err(format!("unknown command {other}: use put, get or sync").into()),
    }
    Ok(())
}
