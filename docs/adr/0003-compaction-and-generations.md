# ADR 0003: Compaction, generations, and the file swap

Status: accepted

## Context

The log only grows, and superseded values pile up in the State Segment.
Compaction must reclaim that space. It must also be crash-safe. A peer that
holds a sync cursor must be able to tell that the log underneath it changed.
Two segments have to be swapped as one unit, and the OS offers no atomic
rename across two files.

## Decisions

### 1. What compaction keeps

Keep the last event for each key, tombstones included. Drop everything
superseded. Tombstones stay because a deletion must still reach a peer that
never saw the live value. Garbage-collecting tombstones needs peer
acknowledgement, which is the open "tombstone GC" question in
[TODO.md](../../TODO.md). It is out of scope here.

Kept events keep their original `lamport` and `node_id`. The clock is carried
over from the old store, so issuing new writes never reuses or rewinds a
timestamp. This matters because conflict order is `(lamport, node_id)`.

### 2. Generations

Each segment header carries a `u32` generation. Compaction produces the
generation after the current one. A cursor is valid only with its generation.
Opening refuses a store whose State and Sync generations differ, so a mixed
pair of files is reported and not silently read.

Peers must store `(generation, cursor)`. A peer that sees a new generation
resynchronizes from the header. The wire protocol for that resync belongs to
Phase 2, so the TODO item "snapshot/resync protocol" stays open. This ADR
settles the semantics it will use: a new generation's log is a complete
replacement that keeps the latest value per key.

### 3. Crash-safe file swap

Files are named by generation: `<base>.state.<gen>` and `<base>.sync`. The
Sync file is the only file that names its generation, so it is the pointer.

1. Write the new state to `<base>.state.<gen+1>`, and the new sync log to
   `<base>.sync.tmp`. Flush both.
2. Rename `<base>.sync.tmp` over `<base>.sync`. **This rename is the commit
   point.**
3. Delete `<base>.state.<gen>`, best-effort.

Each crash point has a clear outcome:

- Before step 2: the old sync file and old state still match. Open finds the
  old generation and deletes the new state file and the temp file.
- After step 2: the new sync file names generation `gen+1`, and its state
  file was flushed before the rename. Open uses it and deletes the old state
  file.

The new store is built by `compact_into` in the core. That step is pure
storage I/O, so it is tested on in-memory segments. The file layer only adds
the ordering and the rename.

### 4. Memory

Compaction holds one copy of each key (in a `BTreeMap`) and the sync-event
list in memory. That is O(keys + events) of heap, so it lives behind the
`alloc` feature. A streaming, `no_alloc` compactor is not designed yet.

## Consequences

- Compaction reclaims the space of superseded values. The sync log shrinks to
  one event per live key, plus tombstones.
- Peers that held old cursors have to resynchronize after compaction. That is
  the cost of compacting a feed that others read.
- Only one process may open a file-backed store at a time. Opening runs
  cleanup and can delete files, so a second writer could race with it. The
  library does not lock files yet.
- On Windows, the old sync file is closed before the rename, because the OS
  will not replace an open file. On Unix the directory is not fsynced after
  the rename, so a power loss can in principle roll the rename back. The
  store then opens the old generation, which is still consistent.
