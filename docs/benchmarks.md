# Benchmarks

The harness is in `bench/`. It is a separate package, outside the workspace,
so the SQLite C library it builds stays out of the core crate.

    cargo run --release --manifest-path bench/Cargo.toml -- <scratch-dir> 20000

Each write is 100 bytes. Durable runs fsync. Each figure is one run on the
development machine (Windows 11, NVMe storage). Run-to-run variance is real, so
ranges are given where the runs disagree.

## Results

| Workload | tpt-hyle | SQLite | Ratio |
|---|---|---|---|
| In-memory, one write per commit | 1.29 M writes/s | 0.31 M writes/s | 4.1x faster |
| File, one durable commit per write | 156 writes/s | 468 writes/s | 0.33x |
| File, batches of 100 | 15.7 k writes/s | 28 to 43 k writes/s | 0.37 to 0.56x |

Reads, with the key index (ADR 0008):

| Keys in log | `get` cost |
|---|---|
| 1,000 | 0.6 µs |
| 10,000 | 0.8 µs |
| 50,000 | 1.2 µs |

Before the index, the same reads took 86 µs, 868 µs and 4.4 ms.

## What the numbers mean

- **Durable writes are bound by fsync.** A commit makes three durable flushes:
  state, data, and the commit record. SQLite's WAL makes one. That accounts
  for most of the file-backed gap.
- **Batching amortizes the cost** but does not close it. Within a batch, both
  engines pay one commit, and our commit still costs three flushes.
- **Closing the gap needs a format change.** One file with one flush per
  commit would match SQLite's commit cost. Two files with checksummed commits
  would cut it to two. Neither is built. See ADR 0002 and ADR 0005.
- **The 10x target was not reached**, and for durable per-commit workloads
  it is not reachable with this design. The in-memory path is faster, and the
  read path is much faster than before the index.

## Reproducing

    cargo run --release --manifest-path bench/Cargo.toml -- <scratch-dir> 20000

Use a scratch directory outside the repository. The harness creates files
there.
