//! KV write benchmark: tpt-hyle against SQLite, same workload.
//!
//! Every write is its own durable commit, the way a typical app writes one
//! record at a time. The file-backed runs fsync. The in-memory runs show the
//! cost of the engine with no disk in the path.
//!
//! Run with: `cargo run --release -- <scratch-dir> [count]`

use std::path::{Path, PathBuf};
use std::time::Instant;

use tpt_hyle::{Hyle, VecStorage};

const VALUE_LEN: usize = 100;

fn key(i: usize) -> Vec<u8> {
    format!("user:{i:08}").into_bytes()
}

fn value(i: usize) -> Vec<u8> {
    let mut v = vec![b'v'; VALUE_LEN];
    v[..8].copy_from_slice(format!("{i:08}").as_bytes());
    v
}

fn report(label: &str, n: usize, secs: f64) {
    println!(
        "{label:<46} {n:>8} writes  {:>10.1} ms  {:>12.0} writes/s",
        secs * 1e3,
        n as f64 / secs
    );
}

fn hyle_mem(n: usize) -> f64 {
    let mut db = Hyle::open(VecStorage::new(), VecStorage::new(), 1).unwrap();
    let start = Instant::now();
    for i in 0..n {
        db.put(&key(i), &value(i)).unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn hyle_file(dir: &Path, n: usize) -> f64 {
    let mut db = Hyle::open_file(dir.join("hyle"), 1).unwrap();
    let start = Instant::now();
    for i in 0..n {
        db.put(&key(i), &value(i)).unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn sqlite_mem(n: usize) -> f64 {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE kv (k BLOB PRIMARY KEY, v BLOB NOT NULL);")
        .unwrap();
    let start = Instant::now();
    for i in 0..n {
        conn.execute(
            "INSERT OR REPLACE INTO kv (k, v) VALUES (?1, ?2)",
            rusqlite::params![key(i), value(i)],
        )
        .unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn sqlite_file(dir: &Path, n: usize) -> f64 {
    let path: PathBuf = dir.join("sqlite.db");
    let _ = std::fs::remove_file(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         CREATE TABLE kv (k BLOB PRIMARY KEY, v BLOB NOT NULL);",
    )
    .unwrap();
    let start = Instant::now();
    for i in 0..n {
        conn.execute(
            "INSERT OR REPLACE INTO kv (k, v) VALUES (?1, ?2)",
            rusqlite::params![key(i), value(i)],
        )
        .unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: tpt-hyle-bench <scratch-dir> [count]"));
    let n: usize = args.next().map_or(20_000, |s| s.parse().unwrap());
    let file_n = n / 10;
    std::fs::create_dir_all(&dir).unwrap();

    println!("workload: {VALUE_LEN}-byte values, one durable commit per write\n");
    report("tpt-hyle, in-memory", n, hyle_mem(n));
    report("SQLite, in-memory", n, sqlite_mem(n));
    println!();
    let hf = hyle_file(&dir, file_n);
    report("tpt-hyle, file (fsync per segment flush)", file_n, hf);
    let sf = sqlite_file(&dir, file_n);
    report("SQLite, file (WAL, synchronous=FULL)", file_n, sf);
    println!("\nfile ratio, tpt-hyle / SQLite throughput: {:.2}x", sf / hf);
    println!();
    batched_report(&dir, file_n);
}

/// Same workload, but `batch` writes per durable commit.
fn hyle_file_batched(dir: &Path, n: usize, batch: usize) -> f64 {
    let mut db = Hyle::open_file(dir.join("hyle-batch"), 1).unwrap();
    let start = Instant::now();
    let mut i = 0;
    while i < n {
        let mut b = db.batch().unwrap();
        for j in i..(i + batch).min(n) {
            b.put(&key(j), &value(j)).unwrap();
        }
        b.commit().unwrap();
        i += batch;
    }
    start.elapsed().as_secs_f64()
}

fn sqlite_file_batched(dir: &Path, n: usize, batch: usize) -> f64 {
    let path: PathBuf = dir.join("sqlite-batch.db");
    let _ = std::fs::remove_file(&path);
    let mut conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         CREATE TABLE kv (k BLOB PRIMARY KEY, v BLOB NOT NULL);",
    )
    .unwrap();
    let start = Instant::now();
    let mut i = 0;
    while i < n {
        let tx = conn.transaction().unwrap();
        for j in i..(i + batch).min(n) {
            tx.execute(
                "INSERT OR REPLACE INTO kv (k, v) VALUES (?1, ?2)",
                rusqlite::params![key(j), value(j)],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        i += batch;
    }
    start.elapsed().as_secs_f64()
}

fn batched_report(dir: &Path, n: usize) {
    let batch = 100;
    report(
        "tpt-hyle, file, batches of 100",
        n,
        hyle_file_batched(dir, n, batch),
    );
    report(
        "SQLite, file, transactions of 100",
        n,
        sqlite_file_batched(dir, n, batch),
    );
}
