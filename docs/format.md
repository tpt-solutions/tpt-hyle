# On-disk format, version 3

Written for: engineers who need to read, verify, or port a store file.

A store is one file. It holds a 128-byte header followed by frames. Every frame
is sealed with XChaCha20-Poly1305, so the file reveals neither keys nor values
without the passphrase. All integers are little-endian. The code is
`crates/tpt-hyle/src/format.rs`; this document must match it.

## Header (128 bytes)

| Offset | Length | Field                | Notes                                          |
|-------:|-------:|----------------------|------------------------------------------------|
| 0      | 4      | magic                | `HYLG`                                         |
| 4      | 2      | version              | `3`                                            |
| 6      | 2      | reserved             | must be zero                                   |
| 8      | 16     | node id              | `u128`, fixed for the life of the store        |
| 24     | 4      | generation           | `u32`, incremented by compaction               |
| 28     | 16     | salt                 | Argon2id salt; all zero for a raw key          |
| 44     | 24     | check nonce          | see *Nonces*                                   |
| 68     | 34     | key-check value      | sealed `"tpt-hyle key check"` (18 + 16 tag)    |
| 102    | 26     | reserved             | must be zero                                   |

The key-check value's associated data is bytes 0..68. A changed node id,
generation, salt, or magic therefore fails the key check, not just the field
that changed.

Opening a store checks, in order: magic, version, reserved bytes, node id
(`NodeIdMismatch`), salt (for a passphrase open, `WrongKey`), and the key-check
value (`WrongKey`). A wrong key never truncates or modifies the file.

## Keys

- **Passphrase:** the key is Argon2id (`argon2` 0.5, default parameters) over
  the passphrase, with the header salt. The salt is random per store and is
  created once. The same passphrase reopens the store.
- **Raw key:** 32 bytes supplied by the caller. The salt is zero.

Argon2id is deliberately slow. Opening a store costs one derivation.

## Frames

Each frame is a 29-byte frame header followed by a sealed payload.

| Offset | Length | Field       | Notes                                              |
|-------:|-------:|-------------|----------------------------------------------------|
| 0      | 4      | sealed_len  | length of the payload, tag included; 16 ≤ len ≤ 2^26 |
| 4      | 1      | kind        | `1` data, `2` commit; anything else is invalid     |
| 5      | 24     | nonce       | the nonce the payload was sealed with              |
| 29     | n      | sealed      | ciphertext followed by the 16-byte Poly1305 tag    |

The associated data for the payload is the kind byte followed by `sealed_len`
(5 bytes). Changing the kind or length fails authentication.

Length bounds are checked before anything is read or allocated.

### Data frame

One write or delete. The plaintext is:

| Offset | Length | Field       | Notes                                   |
|-------:|-------:|-------------|-----------------------------------------|
| 0      | 8      | lamport     | Lamport timestamp of the write          |
| 8      | 16     | node id     | node that originated the write          |
| 24     | 1      | flags       | bit 0 = tombstone (delete)              |
| 25     | 2      | key_len     | at most `MAX_KEY_LEN` (65535)          |
| 27     | 4      | value_len   |                                         |
| 31     | key_len | key        |                                         |
| …      | value_len | value    | empty for a tombstone                   |

The plaintext must be exactly `31 + key_len + value_len` bytes. Any other
length is rejected.

### Commit frame

Closes a batch. The plaintext is 60 bytes:

| Offset | Length | Field    | Notes                                                    |
|-------:|-------:|----------|----------------------------------------------------------|
| 0      | 8      | lamport  | highest Lamport clock at commit                          |
| 8      | 16     | node id  | node that committed                                      |
| 24     | 4      | count    | number of data frames in the batch                       |
| 28     | 32     | digest   | SHA-256 over the full bytes of the batch's data frames   |

The digest covers each data frame's complete bytes: frame header, nonce, and
sealed payload. Nothing in the batch can change without the digest failing.

## Batches and durability

A batch appends its data frames without flushing, then appends one commit frame
and flushes once. The commit frame's count and digest bind the whole batch, so
a batch is committed only when every one of its frames is on disk. One fsync per
commit is therefore sufficient.

A batch that is dropped without a commit truncates the file back to where it
began.

## Recovery

On open, the file is scanned from the header:

1. Each frame must have a valid frame header and authenticate. The first frame
   that does not is the end of the scan.
2. Data frames accumulate. A commit frame must match the accumulated count and
   digest, or the file is `Corrupt`. A commit that verifies makes the batch
   committed.
3. Bytes after the last committed batch are an uncommitted tail. Recovery
   truncates them.
4. If a valid frame appears *after* the scan stopped, the damage is inside
   committed data. Recovery returns `Corrupt` and does not truncate. Truncating
   would discard data that was committed.

Recovery therefore removes a torn write and refuses to remove anything that was
committed.

## Nonces

A nonce is 24 bytes: a 16-byte random session prefix, then a `u64` counter.

- The session prefix is drawn from the system random source at each open.
- The counter starts at zero for each open and only increases. It is not
  rewound when a batch rolls back.

Reusing a nonce under one key exposes plaintext, so the counter must never go
backwards. A rolled-back batch consumes its nonces.

The key-check value uses the session-independent nonce `session ‖ u64::MAX`,
which no data frame reaches.

## Compaction and generations

Compaction writes the live state into a new file, with the generation
incremented, and keeps the original node ids. It writes to `<path>.tmp` and
renames over the original, so a crash leaves either the old file or the new one.
A leftover `<path>.tmp` is harmless and is replaced by the next compaction.

A peer's sync cursor is `(generation, offset)`. It is valid only for the
generation it was taken from. A cursor from an older generation is detectably
stale, so the holder must not resume from its offset.

## Errors

| Error                      | Meaning                                             |
|----------------------------|-----------------------------------------------------|
| `WrongKey`                 | passphrase or key does not match; file is untouched |
| `NodeIdMismatch`           | file belongs to a different node id                 |
| `Corrupt(reason)`          | damage inside committed data, or a malformed header |
| `Crypto(reason)`           | key derivation or randomness failed                 |

## Verification

- Kani proofs cover header and frame decoding never panicking, frame lengths
  staying in bounds, data and commit decoding never panicking, and fixed-storage
  reads and truncation staying in bounds. Harness names are in `kani/README.md`.
- Engine tests cover torn commits, uncommitted tails, damage inside committed
  data, wrong keys, nonce continuity across rollbacks, and compaction.
- The `tpt-hyle-cli verify` command authenticates every frame of a store.

## Not in version 3

- No per-event signatures. Authorship is the node id, and any holder of the key
  can write as any node. Signed events are planned.
- No tombstone collection. Compaction keeps the latest event per key, including
  tombstones, so a deleted key stays deleted on peers that have not yet seen the
  delete. Tombstone garbage collection with peer acknowledgements is planned.
- The format does not hide the number of frames, their sizes, or their timing.
  Those leak through the file size and the frame count.
