# Benchmarks

The harness is in `bench/`. It is a separate package, outside the workspace,
so the SQLite C library it builds stays out of the core crate.

    cargo run --release --manifest-path bench/Cargo.toml -- <scratch-dir> 20000

Each write is 100 bytes. Durable runs fsync. Each figure is one run on the
development machine (Windows 11, NVMe storage). Run-to-run variance is real, so
treat these as one sample, not a guarantee.

These numbers are for format version 4, which signs every event with Ed25519.

## Results

| Workload | tpt-hyle | SQLite | Ratio |
|---|---|---|---|
| In-memory, one write per commit | 24.2 k writes/s | 285 k writes/s | 0.085x |
| File, one durable commit per write | 461 writes/s | 450 writes/s | 1.03x |
| File, batches of 100 | 17.2 k writes/s | 38.7 k writes/s | 0.44x |

Reads, through the key index (ADR 0008):

| Keys in log | `get` cost |
|---|---|
| 1,000 | 4.1 µs |
| 10,000 | 5.0 µs |
| 50,000 | 5.9 µs |

## What changed, and why it matters

Format version 3 wrote encrypted frames with no signatures. An earlier run on
the same machine measured it at about 275 k in-memory writes per second. Version
4 adds one Ed25519 signature to every write. That costs about 35 µs per write,
which is most of the in-memory time. The durable one-commit-per-write path is
unchanged, because it is bound by fsync and the signature hides inside that
wait. The batched path also pays the signature on every write.

So the in-memory engine is now roughly 11 times slower than before signing, and
it is 12 times slower than SQLite in memory. This is the price of per-event
signatures. The signature is what lets a relay forward an event, and lets a
peer check it, without trusting the relay. See [ADR 0015](adr/0015-signed-events.md).

Signature verification costs more, because the receiver checks each change it
imports. The bench does not measure that path yet.

## What the numbers mean

- **Durable writes are bound by fsync.** A commit makes one durable flush
  (`sync_data` on the single file), which matches SQLite WAL's one.
- **Batching amortizes the commit flush,** but with signing, the per-write cost
  dominates a batch. A batch of 100 is about 0.44x SQLite here.
- **Reads are a few microseconds,** and grow slowly with the log, because the
  key index covers the lookup.
- **In-memory writes are now signature-bound.** A future change could reduce
  this by signing a batch rather than each event, but relayed events need their
  origin's signature, so that would need a different scheme.

## Not measured

- Import and verification throughput on a receiver.
- The sync path end to end.
- Encrypted-read cost against the unencrypted layout of earlier versions.

## Reproducing

    cargo run --release --manifest-path bench/Cargo.toml -- <scratch-dir> 20000

Use a scratch directory outside the repository. The harness creates files
there.
