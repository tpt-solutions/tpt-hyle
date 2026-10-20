//! `tpt-hyle-cli`: inspect, dump, verify and compact tpt-hyle stores.
//!
//! Every command needs the store's passphrase in the `TPT_HYLE_PASSPHRASE`
//! environment variable, so it does not end up in shell history or a file.
//!
//! Opening a store runs recovery, which removes an uncommitted tail. Run these
//! commands on a copy if the file must stay byte-identical.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tpt_hyle::{file_node_id, ChangeEvent, FileStorage, Hyle, HEADER_LEN};

type Store = Hyle<FileStorage>;
type CliResult = Result<(), Box<dyn StdError>>;

/// Environment variable that carries the passphrase.
const PASSPHRASE_VAR: &str = "TPT_HYLE_PASSPHRASE";

#[derive(Parser)]
#[command(
    name = "tpt-hyle-cli",
    version,
    about = "Inspect, dump, verify and compact tpt-hyle stores"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarise a store: node id, clock, generation, sizes, and live keys.
    Inspect {
        /// Store file path.
        path: PathBuf,
    },
    /// Print every write in order.
    DumpSyncLog {
        /// Store file path.
        path: PathBuf,
        /// Emit one JSON object per line. Values are shown as lossy UTF-8.
        #[arg(long)]
        json: bool,
    },
    /// Read and authenticate every frame.
    Verify {
        /// Store file path.
        path: PathBuf,
    },
    /// Print the store's Ed25519 public key. List it in the server config
    /// under the client's token, so the hub accepts this node's events.
    PublicKey {
        /// Store file path.
        path: PathBuf,
    },
    /// Drop superseded values. Starts a new generation, so peer cursors from
    /// the old generation become stale.
    Compact {
        /// Store file path.
        path: PathBuf,
    },
}

/// One write, with its key and value copied out of the store.
struct Entry {
    offset: u64,
    event: ChangeEvent,
    key: Vec<u8>,
    value: Vec<u8>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Inspect { path } => inspect(&path),
        Command::DumpSyncLog { path, json } => dump(&path, json),
        Command::Verify { path } => verify(&path),
        Command::PublicKey { path } => public_key(&path),
        Command::Compact { path } => compact(&path),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn passphrase() -> Result<String, Box<dyn StdError>> {
    std::env::var(PASSPHRASE_VAR)
        .map_err(|_| format!("set {PASSPHRASE_VAR} to the store passphrase").into())
}

fn open(path: &Path) -> Result<Store, Box<dyn StdError>> {
    let node_id = file_node_id(path)?;
    Ok(Hyle::open_file(path, node_id, passphrase()?.as_bytes())?)
}

/// Reads every write with its key and value. Authenticates each frame.
fn read_all(store: &mut Store) -> Result<Vec<Entry>, Box<dyn StdError>> {
    let events: Vec<(u64, ChangeEvent)> =
        store.events_since(HEADER_LEN)?.collect::<Result<_, _>>()?;
    let mut entries = Vec::with_capacity(events.len());
    for (_, event) in events {
        let mut key = vec![0u8; event.key_len as usize];
        let mut value = vec![0u8; event.payload_len as usize];
        store.read_record(&event, &mut key, &mut value)?;
        entries.push(Entry {
            offset: event.payload_offset,
            event,
            key,
            value,
        });
    }
    Ok(entries)
}

fn inspect(path: &Path) -> CliResult {
    let mut store = open(path)?;
    let entries = read_all(&mut store)?;

    let mut live: HashMap<&[u8], bool> = HashMap::new();
    for e in &entries {
        live.insert(&e.key, !e.event.tombstone);
    }
    let live_keys = live.values().filter(|&&alive| alive).count();
    let tombstones = entries.iter().filter(|e| e.event.tombstone).count();

    println!("node id:     {:#034x}", store.node_id());
    println!("public key:  {}", hex(&store.public_key()));
    println!("generation:  {}", store.generation());
    println!("lamport:     {}", store.lamport());
    println!("writes:      {}", entries.len());
    println!("tombstones:  {tombstones}");
    println!("live keys:   {live_keys}");
    println!("file bytes:  {}", store.sync_end());
    Ok(())
}

fn dump(path: &Path, json: bool) -> CliResult {
    let mut store = open(path)?;
    for e in read_all(&mut store)? {
        let op = if e.event.tombstone { "delete" } else { "put" };
        let key = String::from_utf8_lossy(&e.key);
        let value = String::from_utf8_lossy(&e.value);
        if json {
            let line = serde_json::json!({
                "offset": e.offset,
                "lamport": e.event.lamport,
                "node_id": format!("{:#034x}", e.event.node_id),
                "op": op,
                "key": key,
                "value": value,
                "value_len": e.value.len(),
            });
            println!("{line}");
        } else {
            println!(
                "{:>10}  {:>8}  {:#034x}  {:<6}  {}  ({} bytes)",
                e.offset,
                e.event.lamport,
                e.event.node_id,
                op,
                key,
                e.value.len()
            );
        }
    }
    Ok(())
}

fn verify(path: &Path) -> CliResult {
    let mut store = open(path)?;
    let entries = read_all(&mut store)?;
    println!("ok: {} writes verified", entries.len());
    Ok(())
}

fn public_key(path: &Path) -> CliResult {
    let store = open(path)?;
    println!("{}", hex(&store.public_key()));
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn compact(path: &Path) -> CliResult {
    let store = open(path)?;
    let (gen_before, len_before) = (store.generation(), store.sync_end());
    let store = store.compact_file(path)?;
    println!(
        "compacted: generation {gen_before} -> {}, file {len_before} -> {} bytes",
        store.generation(),
        store.sync_end()
    );
    Ok(())
}
