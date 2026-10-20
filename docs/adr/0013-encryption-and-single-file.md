# ADR 0013: Encryption at rest, single file, one fsync per commit

Status: accepted

## Context

Version 2 stored plaintext in a directory of segment files, checksummed with
FNV-1a, and needed an fsync per segment. Three things were wrong with it:

- Anyone with the disk could read every value. Checksums detect damage; they
  do not hide data.
- Durable writes were slower than SQLite WAL, because each commit synced more
  than once.
- A store was several files, so copying, backing up, or deleting one meant
  handling a set.

Keeping a `no_std` storage-only build alongside the encrypted engine meant two
engines to maintain. Both were needed for the same job.

## Decision

Version 3 is one encrypted, append-only file. See `docs/format.md` for the
layout.

- Every frame is sealed with XChaCha20-Poly1305. Keys come from a passphrase
  via Argon2id, or a caller-supplied raw 32-byte key.
- A batch is committed by one commit frame that carries a count and a SHA-256
  digest of the batch. A batch is durable only when its commit verifies, so one
  fsync per commit is enough.
- Recovery truncates a torn tail. It refuses to truncate when a valid frame
  follows the stop point, since that means committed data is damaged.
- Nonces are a random per-open session prefix plus a counter that never rewinds,
  including across rollbacks.
- The engine requires `alloc`. The `no_alloc` store is removed. Its
  `FixedStorage` remains as a `Storage` implementation, so the no_std build
  still checks on a bare-metal target.

## Consequences

- The file is unreadable without the passphrase. Losing the passphrase loses
  the data. There is no recovery path.
- Each frame costs an AEAD operation and a SHA-256 update. In-memory write
  throughput fell from about 1.29 M/s to about 275 k/s. Durable write throughput
  is at parity with SQLite WAL. Read latency went from about 1 µs to 3–4 µs.
  `docs/benchmarks.md` has the numbers.
- Opening a store runs one Argon2id derivation, which is intentionally slow.
- The file does not hide sizes, counts, or timing.
- Format version 2 files are not readable by version 3 code. No migration tool
  exists.

## MSRV scope

The workspace no longer has one MSRV. The core crate `tpt-hyle` and the
replication crate `tpt-rift` build on Rust 1.75. The rest of the workspace
(transports, server, bindings) needs a current stable toolchain. Its
dependencies, including the WebRTC stack, the ICU crates, and UniFFI, need
newer compilers than 1.75. `zeroize` and `base64ct` are pinned
to versions that build on 1.75, so the core stays on that floor. Those pins
are a maintenance cost and must be revisited when they block a dependency
update.

## Alternatives rejected

- **Per-file encryption with a manifest.** Keeps the multi-file layout, but
  needs its own atomicity story. One file with commit frames gets atomicity
  from the format.
- **Encrypting only values.** Leaks keys and the write history. Sealing every
  frame costs little more and hides the structure.
- **Keeping the plaintext engine as an option.** Two engines with different
  guarantees invite using the weaker one by mistake.

## Related

- [[0002-integrity-and-indexing]] — the checksum scheme this replaces.
- [[0005-batched-commits]] — batch semantics, which the commit frame now binds.
- [[0008-key-index]] — the index is rebuilt from the file on open.
