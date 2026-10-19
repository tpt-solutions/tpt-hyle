//! `tpt-hyle-cli`: inspect, dump, verify and compact tpt-hyle stores.
//!
//! Opening a store runs recovery, which truncates a torn tail. Run these
//! commands on a copy if the files must stay byte-identical.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tpt_hyle::{file_node_id, ChangeEvent, FileStorage, Hyle, HEADER_LEN, SYNC_RECORD_LEN};

type Store = Hyle<FileStorage, FileStorage>;
type CliResult = Result<(), Box<dyn StdError>>;

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
        /// Store base path, as passed to `Hyle::open_file`.
        base: PathBuf,
    },
    /// Print every sync event in order.
    DumpSyncLog {
        /// Store base path.
        base: PathBuf,
        /// Emit one JSON object per line. Values are shown as lossy UTF-8.
        #[arg(long)]
        json: bool,
    },
    /// Read every record and check every checksum.
    Verify {
        /// Store base path.
        base: PathBuf,
    },
    /// Drop superseded values. Starts a new generation, so peer cursors from
    /// the old generation become stale.
    Compact {
        /// Store base path.
        base: PathBuf,
    },
}

/// One sync event with its key and value copied out of the State Segment.
struct Entry {
    offset: u64,
    event: ChangeEvent,
    key: Vec<u8>,
    value: Vec<u8>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Inspect { base } => inspect(&base),
        Command::DumpSyncLog { base, json } => dump(&base, json),
        Command::Verify { base } => verify(&base),
        Command::Compact { base } => compact(&base),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(base: &Path) -> Result<Store, Box<dyn StdError>> {
    let node_id = file_node_id(base)?;
    Ok(Hyle::open_file(base, node_id)?)
}

/// Reads every sync event with its key and value. Checks each state record.
fn read_all(store: &mut Store) -> Result<Vec<Entry>, Box<dyn StdError>> {
    let events: Vec<(u64, ChangeEvent)> =
        store.events_since(HEADER_LEN)?.collect::<Result<_, _>>()?;
    let mut entries = Vec::with_capacity(events.len());
    for (end, event) in events {
        let mut key = vec![0u8; event.key_len as usize];
        let mut value = vec![0u8; event.payload_len as usize];
        store.read_record(&event, &mut key, &mut value)?;
        entries.push(Entry {
            offset: end - SYNC_RECORD_LEN as u64,
            event,
            key,
            value,
        });
    }
    Ok(entries)
}

fn inspect(base: &Path) -> CliResult {
    let mut store = open(base)?;
    let entries = read_all(&mut store)?;

    let mut live: HashMap<&[u8], bool> = HashMap::new();
    for e in &entries {
        live.insert(&e.key, !e.event.tombstone);
    }
    let live_keys = live.values().filter(|&&alive| alive).count();
    let tombstones = entries.iter().filter(|e| e.event.tombstone).count();

    println!("node id:     {:#034x}", store.node_id());
    println!("generation:  {}", store.generation());
    println!("lamport:     {}", store.lamport());
    println!("sync events: {}", entries.len());
    println!("tombstones:  {tombstones}");
    println!("live keys:   {live_keys}");
    println!("sync bytes:  {}", store.sync_end());
    println!("state bytes: {}", store.state_len());
    Ok(())
}

fn dump(base: &Path, json: bool) -> CliResult {
    let mut store = open(base)?;
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

fn verify(base: &Path) -> CliResult {
    let mut store = open(base)?;
    let entries = read_all(&mut store)?;
    println!("ok: {} sync events verified", entries.len());
    Ok(())
}

fn compact(base: &Path) -> CliResult {
    let store = open(base)?;
    let (gen_before, sync_before, state_before) =
        (store.generation(), store.sync_end(), store.state_len());
    let store = store.compact_file(base)?;
    println!(
        "compacted: generation {gen_before} -> {}, sync {sync_before} -> {} bytes, state {state_before} -> {} bytes",
        store.generation(),
        store.sync_end(),
        store.state_len()
    );
    Ok(())
}
