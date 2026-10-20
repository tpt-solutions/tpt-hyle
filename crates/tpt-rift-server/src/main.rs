//! `tpt-rift-server`: run the sync server, or manage client tokens.
//!
//! ```text
//! tpt-rift-server serve <config.json>
//! tpt-rift-server collect <config.json>     (server stopped)
//! tpt-rift-server new-token
//! tpt-rift-server hash-token <token>
//! ```
//!
//! `serve` and `collect` read the store passphrase from `TPT_HUB_PASSPHRASE`.

use std::path::PathBuf;
use std::process::ExitCode;

use tpt_rift_server::{collect_tombstones, hash_token, load_config, new_token, run};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result: Result<(), Box<dyn std::error::Error>> = match args.first().map(String::as_str) {
        Some("serve") => match args.get(1) {
            Some(path) => match load_config(&PathBuf::from(path)) {
                Ok(config) => match std::env::var("TPT_HUB_PASSPHRASE") {
                    Ok(pass) => run(config, pass.as_bytes()).await,
                    Err(_) => Err("set TPT_HUB_PASSPHRASE to the hub store passphrase".into()),
                },
                Err(e) => Err(Box::new(e)),
            },
            None => Err("usage: tpt-rift-server serve <config.json>".into()),
        },
        Some("collect") => match args.get(1) {
            Some(path) => match load_config(&PathBuf::from(path)) {
                Ok(config) => match std::env::var("TPT_HUB_PASSPHRASE") {
                    Ok(pass) => collect_tombstones(&config, pass.as_bytes()).map(|c| {
                        match c.bound {
                            Some(b) => println!(
                                "collected with bound offset {} lamport {}",
                                b.offset, b.lamport
                            ),
                            None => println!(
                                "nothing collected: not every client has acked the current generation"
                            ),
                        }
                        println!(
                            "generation {} -> {}",
                            c.generations.0, c.generations.1
                        );
                    }),
                    Err(_) => Err("set TPT_HUB_PASSPHRASE to the hub store passphrase".into()),
                },
                Err(e) => Err(Box::new(e)),
            },
            None => Err("usage: tpt-rift-server collect <config.json>".into()),
        },
        Some("new-token") => {
            let (token, hash) = new_token();
            println!("token (give this to the client once): {token}");
            println!("sha256 (put this in the config):      {hash}");
            Ok(())
        }
        Some("hash-token") => match args.get(1) {
            Some(token) => {
                println!("{}", hash_token(token));
                Ok(())
            }
            None => Err("usage: tpt-rift-server hash-token <token>".into()),
        },
        _ => Err(
            "usage: tpt-rift-server serve <config.json> | collect <config.json> | new-token | hash-token <token>".into(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
