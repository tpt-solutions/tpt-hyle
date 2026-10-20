//! The dual-segment engine: batched writes, `get`, the sync log cursor,
//! crash recovery on open, and compaction.
//!
//! Every write belongs to a batch. A batch appends its state and sync records
//! without flushing, then `commit` makes them durable in this order: state
//! records, sync records, then one commit record. A batch with no commit
//! record is not part of the store, and recovery removes it.

use core::cmp::{max, min};

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::checksum::{crc32c_finish, crc32c_update, fnv1a, fnv1a_update, CRC_INIT, FNV_OFFSET};
use crate::format::{
    check_header, decode_record, encode_commit, encode_header, le_u16, le_u32, ChangeEvent, Record,
    HEADER_LEN, MAX_KEY_LEN, STATE_MAGIC, STATE_RECORD_HEADER_LEN, SYNC_MAGIC, SYNC_RECORD_LEN,
};
#[cfg(feature = "std")]
use crate::storage::FileStorage;
use crate::storage::Storage;
use crate::Error;

/// Scratch size for streaming reads of state records.
const CHUNK: usize = 64;
const REC: u64 = SYNC_RECORD_LEN as u64;

/// A key and its value, as returned by [`Hyle::live_entries`].
#[cfg(feature = "alloc")]
pub type KeyValue = (Vec<u8>, Vec<u8>);

/// A sync-native key-value store over a State and a Sync segment.
///
/// Every write appends a state record, then a sync event that points at it.
/// The sync log is the replication feed: a peer reads it from its cursor.
///
/// Each store has a *generation*. Compaction starts a new generation, which
/// invalidates any cursor a peer holds from the old one. See
/// [`Hyle::generation`].
///
/// Reads take `&mut self` because the storage backends may seek. A failed
/// write poisons the store until it is reopened.
pub struct Hyle<St, Sy> {
    state: St,
    sync: Sy,
    node_id: u128,
    generation: u32,
    lamport: u64,
    failed: bool,
    /// Sync offsets of data records, per key hash, oldest first. Rebuilt on
    /// open. Only present with `alloc`. Without it, lookups scan the log.
    #[cfg(feature = "alloc")]
    index: BTreeMap<u64, Vec<u64>>,
}

/// A group of writes that commits or disappears together.
///
/// Writes are visible to later calls on the same batch, and to `get` once
/// the batch is committed. Dropping an uncommitted batch rolls back every
/// record it appended, so the store is left as it was before the batch.
///
/// ```
/// use tpt_hyle::{Hyle, VecStorage};
///
/// let mut db = Hyle::open(VecStorage::new(), VecStorage::new(), 1)?;
/// let mut batch = db.batch()?;
/// batch.put(b"a", b"1")?;
/// batch.put(b"b", b"2")?;
/// batch.commit()?; // both writes land, or neither does
/// # Ok::<(), tpt_hyle::Error>(())
/// ```
pub struct Batch<'a, St: Storage, Sy: Storage> {
    store: &'a mut Hyle<St, Sy>,
    start_state: u64,
    start_sync: u64,
    start_lamport: u64,
    pending: usize,
    committed: bool,
}

impl<St: Storage, Sy: Storage> Hyle<St, Sy> {
    /// Opens a store over the two segments, creating them if they are empty.
    ///
    /// Recovery runs here. It keeps every record up to the last valid commit
    /// record, and removes what follows. A damaged record before that point
    /// is reported as [`Error::Corrupt`]. State bytes that no committed event
    /// points at are discarded.
    pub fn open(state: St, sync: Sy, node_id: u128) -> Result<Self, Error> {
        Self::open_with(state, sync, node_id, 0)
    }

    /// Opens over segments that may be empty. `fresh_generation` is written
    /// only into segments that are being created.
    fn open_with(
        mut state: St,
        mut sync: Sy,
        node_id: u128,
        fresh_generation: u32,
    ) -> Result<Self, Error> {
        let state_gen = init_segment(&mut state, STATE_MAGIC, node_id, fresh_generation)?;
        let sync_gen = init_segment(&mut sync, SYNC_MAGIC, node_id, fresh_generation)?;
        if state_gen != sync_gen {
            return Err(Error::Corrupt(
                "state and sync segments belong to different generations",
            ));
        }
        let mut store = Self {
            state,
            sync,
            node_id,
            generation: sync_gen,
            lamport: 0,
            failed: false,
            #[cfg(feature = "alloc")]
            index: BTreeMap::new(),
        };
        store.lamport = store.recover()?;
        Ok(store)
    }

    /// The node id this store writes under.
    pub fn node_id(&self) -> u128 {
        self.node_id
    }

    /// The generation of this store's segments.
    ///
    /// A generation is a number that compaction increments. A peer that
    /// stores a sync cursor must also store the generation it came from. If
    /// the generation changes, the cursor is stale and the peer has to
    /// resynchronize.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The highest Lamport timestamp issued or observed so far.
    pub fn lamport(&self) -> u64 {
        self.lamport
    }

    /// Advances the clock past a timestamp seen from a peer. Only in memory
    /// until the next commit, which records the clock.
    pub fn observe_lamport(&mut self, remote: u64) {
        self.lamport = max(self.lamport, remote);
    }

    /// Starts a batch. Fails if the store is poisoned.
    pub fn batch(&mut self) -> Result<Batch<'_, St, Sy>, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        Ok(Batch {
            start_state: self.state.len(),
            start_sync: self.sync.len(),
            start_lamport: self.lamport,
            pending: 0,
            committed: false,
            store: self,
        })
    }

    /// Writes `value` under `key` in a batch of one, and returns its event.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<ChangeEvent, Error> {
        let mut batch = self.batch()?;
        let ev = batch.put(key, value)?;
        batch.commit()?;
        Ok(ev)
    }

    /// Deletes `key`. Returns `false` if the key was not present.
    pub fn delete(&mut self, key: &[u8]) -> Result<bool, Error> {
        let mut batch = self.batch()?;
        let removed = batch.delete(key)?;
        batch.commit()?;
        Ok(removed)
    }

    /// Appends a write from another node, keeping its Lamport timestamp,
    /// node id and tombstone flag, in a batch of one.
    ///
    /// This does not compare against the current value. The caller decides
    /// whether the write should win. The local clock advances to cover
    /// `lamport`.
    pub fn put_remote(
        &mut self,
        key: &[u8],
        value: &[u8],
        lamport: u64,
        node_id: u128,
        tombstone: bool,
    ) -> Result<ChangeEvent, Error> {
        let mut batch = self.batch()?;
        let ev = batch.put_remote(key, value, lamport, node_id, tombstone)?;
        batch.commit()?;
        Ok(ev)
    }

    /// Copies the value for `key` into `out` and returns its length.
    ///
    /// Returns `Ok(None)` if the key is absent or deleted. Returns
    /// [`Error::BufferTooSmall`] if `out` cannot hold the value.
    pub fn get(&mut self, key: &[u8], out: &mut [u8]) -> Result<Option<usize>, Error> {
        check_key(key)?;
        let ev = match self.find(key)? {
            Some(ev) if !ev.tombstone => ev,
            _ => return Ok(None),
        };
        let n = ev.payload_len as usize;
        if out.len() < n {
            return Err(Error::BufferTooSmall { needed: n });
        }
        self.read_value(&ev, &mut out[..n])?;
        Ok(Some(n))
    }

    /// `true` if `key` holds a live value.
    pub fn contains_key(&mut self, key: &[u8]) -> Result<bool, Error> {
        check_key(key)?;
        Ok(matches!(self.find(key)?, Some(ev) if !ev.tombstone))
    }

    /// Copies the key and value of a sync event into the caller's buffers.
    ///
    /// Returns the key and value lengths. The record is checksum-verified
    /// first, so a corrupt payload is reported rather than returned.
    pub fn read_record(
        &mut self,
        ev: &ChangeEvent,
        key_out: &mut [u8],
        value_out: &mut [u8],
    ) -> Result<(usize, usize), Error> {
        let kl = ev.key_len as usize;
        let vl = ev.payload_len as usize;
        if key_out.len() < kl {
            return Err(Error::BufferTooSmall { needed: kl });
        }
        if value_out.len() < vl {
            return Err(Error::BufferTooSmall { needed: vl });
        }
        self.check_state_record(ev)?;
        self.state.read_at(
            ev.payload_offset + STATE_RECORD_HEADER_LEN,
            &mut key_out[..kl],
        )?;
        self.read_value(ev, &mut value_out[..vl])?;
        Ok((kl, vl))
    }

    /// Iterates the write events in the sync log from `cursor`. The cursor is
    /// a byte offset: either [`HEADER_LEN`] or the value returned by a
    /// previous item.
    ///
    /// Each item is `(next_cursor, event)`. Commit records are skipped. Store
    /// `next_cursor` to resume.
    pub fn events_since(&mut self, cursor: u64) -> Result<Events<'_, Sy>, Error> {
        let end = self.sync.len();
        if cursor < HEADER_LEN || cursor > end || (cursor - HEADER_LEN) % REC != 0 {
            return Err(Error::BadCursor(cursor));
        }
        Ok(Events {
            sync: &mut self.sync,
            offset: cursor,
            end,
        })
    }

    /// Current end of the sync log. A peer that has seen everything up to
    /// here can use this as its cursor.
    pub fn sync_end(&self) -> u64 {
        self.sync.len()
    }

    /// Current length of the State Segment, in bytes. Includes superseded
    /// values until compaction removes them.
    pub fn state_len(&self) -> u64 {
        self.state.len()
    }

    /// Returns the two segments, consuming the store. Useful for reopening
    /// and for fault-injection tests.
    pub fn segments(&self) -> (&St, &Sy) {
        (&self.state, &self.sync)
    }

    /// Returns the two segments, consuming the store.
    pub fn into_parts(self) -> (St, Sy) {
        (self.state, self.sync)
    }

    /// Rewrites the store into `state` and `sync`, which must be empty.
    ///
    /// The new store keeps the latest event for each key, tombstones
    /// included. Superseded values are dropped. Each kept event keeps its
    /// original Lamport timestamp and node id, and the clock never moves
    /// backwards. The new store has the next generation, so old cursors are
    /// detectably stale. The copy is one batch.
    ///
    /// This method does not swap files. [`Hyle::compact_file`] does the
    /// crash-safe swap for file-backed stores.
    #[cfg(feature = "alloc")]
    pub fn compact_into<St2: Storage, Sy2: Storage>(
        &mut self,
        state: St2,
        sync: Sy2,
    ) -> Result<Hyle<St2, Sy2>, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        if !state.is_empty() || !sync.is_empty() {
            return Err(Error::TargetNotEmpty);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(Error::Corrupt("generation counter exhausted"))?;

        let events: Vec<ChangeEvent> = self
            .events_since(HEADER_LEN)?
            .map(|item| item.map(|(_, ev)| ev))
            .collect::<Result<_, _>>()?;

        // Pass 1: find each key's last event, and check every state record
        // up front so that copying cannot silently propagate damage.
        let mut keys: Vec<Vec<u8>> = Vec::with_capacity(events.len());
        let mut latest: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        for (i, ev) in events.iter().enumerate() {
            self.check_state_record(ev)?;
            let mut key = vec![0u8; ev.key_len as usize];
            self.state
                .read_at(ev.payload_offset + STATE_RECORD_HEADER_LEN, &mut key)?;
            latest.insert(key.clone(), i);
            keys.push(key);
        }

        // Pass 2: copy the surviving events in their original order, as one
        // batch. The clock goes in the commit record, so it survives even
        // when no events survive.
        let mut target = Hyle::open_with(state, sync, self.node_id, generation)?;
        target.observe_lamport(self.lamport);
        let mut batch = target.batch()?;
        for (i, ev) in events.iter().enumerate() {
            if latest.get(&keys[i]) != Some(&i) {
                continue;
            }
            let mut value = vec![0u8; ev.payload_len as usize];
            self.read_value(ev, &mut value)?;
            batch.put_remote(&keys[i], &value, ev.lamport, ev.node_id, ev.tombstone)?;
        }
        if self.lamport > 0 {
            batch.commit_forced()?;
        } else {
            batch.commit()?;
        }
        Ok(target)
    }

    /// Every live key that starts with `prefix`, with its value, in key order.
    ///
    /// Scans the whole log, so the cost is linear in its length. Each key's
    /// latest event decides whether it is live.
    #[cfg(feature = "alloc")]
    pub fn live_entries(&mut self, prefix: &[u8]) -> Result<Vec<KeyValue>, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        let events: Vec<ChangeEvent> = self
            .events_since(HEADER_LEN)?
            .map(|item| item.map(|(_, ev)| ev))
            .collect::<Result<_, _>>()?;

        let mut latest: BTreeMap<Vec<u8>, Option<ChangeEvent>> = BTreeMap::new();
        for ev in events {
            self.check_state_record(&ev)?;
            let mut key = vec![0u8; ev.key_len as usize];
            self.state
                .read_at(ev.payload_offset + STATE_RECORD_HEADER_LEN, &mut key)?;
            latest.insert(key, if ev.tombstone { None } else { Some(ev) });
        }

        let mut out = Vec::new();
        for (key, ev) in latest {
            if !key.starts_with(prefix) {
                continue;
            }
            if let Some(ev) = ev {
                let mut value = vec![0u8; ev.payload_len as usize];
                self.read_value(&ev, &mut value)?;
                out.push((key, value));
            }
        }
        Ok(out)
    }

    /// Copies the value for `key` into a new vector. `None` if absent or
    /// deleted.
    #[cfg(feature = "json")]
    pub(crate) fn get_vec(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        check_key(key)?;
        let ev = match self.find(key)? {
            Some(ev) if !ev.tombstone => ev,
            _ => return Ok(None),
        };
        let mut value = vec![0u8; ev.payload_len as usize];
        self.read_value(&ev, &mut value)?;
        Ok(Some(value))
    }

    /// Latest event for `key`, tombstones included. Replication uses it to
    /// decide whether an incoming write wins.
    pub fn latest_event(&mut self, key: &[u8]) -> Result<Option<ChangeEvent>, Error> {
        check_key(key)?;
        self.find(key)
    }

    /// Appends one write without flushing. Poisons the store on failure,
    /// because a half-written record may now sit in the log.
    fn write_pending(
        &mut self,
        key: &[u8],
        value: &[u8],
        tombstone: bool,
        lamport: u64,
        node_id: u128,
    ) -> Result<ChangeEvent, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        match self.append_unsealed(key, value, tombstone, lamport, node_id) {
            Ok(ev) => Ok(ev),
            Err(e) => {
                self.failed = true;
                Err(e)
            }
        }
    }

    fn append_unsealed(
        &mut self,
        key: &[u8],
        value: &[u8],
        tombstone: bool,
        lamport: u64,
        node_id: u128,
    ) -> Result<ChangeEvent, Error> {
        let key_len = key.len() as u16;
        let value_len = value.len() as u32;
        let kl = key_len.to_le_bytes();
        let vl = value_len.to_le_bytes();

        let mut header = [0u8; STATE_RECORD_HEADER_LEN as usize];
        header[4..6].copy_from_slice(&kl);
        header[6..10].copy_from_slice(&vl);
        let crc = record_crc(&header[4..10], key, value);
        header[0..4].copy_from_slice(&crc.to_le_bytes());

        let payload_offset = self.state.append(&header)?;
        self.state.append(key)?;
        self.state.append(value)?;

        let ev = ChangeEvent {
            key_hash: fnv1a(key),
            key_len,
            lamport,
            node_id,
            payload_offset,
            payload_len: value_len,
            tombstone,
        };
        let offset = self.sync.append(&ev.encode())?;
        self.index_add(ev.key_hash, offset);
        Ok(ev)
    }

    #[cfg(feature = "alloc")]
    fn index_add(&mut self, key_hash: u64, offset: u64) {
        self.index.entry(key_hash).or_default().push(offset);
    }

    #[cfg(not(feature = "alloc"))]
    fn index_add(&mut self, _key_hash: u64, _offset: u64) {}

    /// Forgets entries at or after `from`. Used when a batch rolls back.
    #[cfg(feature = "alloc")]
    fn index_retract(&mut self, from: u64) {
        self.index.retain(|_, offsets| {
            offsets.retain(|&o| o < from);
            !offsets.is_empty()
        });
    }

    #[cfg(not(feature = "alloc"))]
    fn index_retract(&mut self, _from: u64) {}

    /// Makes a batch durable. The order is state records, then sync records,
    /// then the commit record, with a flush between each step. A commit
    /// record therefore never persists ahead of the data it closes.
    fn seal(&mut self) -> Result<(), Error> {
        let result = self.write_commit();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn write_commit(&mut self) -> Result<(), Error> {
        self.state.flush()?;
        self.sync.flush()?;
        let marker = encode_commit(self.lamport, self.node_id);
        self.sync.append(&marker)?;
        self.sync.flush()?;
        Ok(())
    }

    /// The latest event on `key`, including writes made earlier in the
    /// current batch. Refuses on a poisoned store, because a failed commit
    /// can leave records that a reopen would discard.
    fn find(&mut self, key: &[u8]) -> Result<Option<ChangeEvent>, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        self.find_latest(key)
    }

    /// Looks up the latest event through the key index. Each candidate is
    /// checked against the full key, so a hash collision cannot return the
    /// wrong value.
    #[cfg(feature = "alloc")]
    fn find_latest(&mut self, key: &[u8]) -> Result<Option<ChangeEvent>, Error> {
        let key_hash = fnv1a(key);
        let count = self.index.get(&key_hash).map_or(0, Vec::len);
        for i in (0..count).rev() {
            let Some(&offset) = self.index.get(&key_hash).and_then(|v| v.get(i)) else {
                break;
            };
            if let Record::Data(ev) = self.read_record_at(offset)? {
                if ev.key_len as usize == key.len() && self.matches_key(&ev, key)? {
                    return Ok(Some(ev));
                }
            }
        }
        Ok(None)
    }

    /// Scans the sync log newest-first. Used without `alloc`, where there is
    /// no index.
    #[cfg(not(feature = "alloc"))]
    fn find_latest(&mut self, key: &[u8]) -> Result<Option<ChangeEvent>, Error> {
        let key_hash = fnv1a(key);
        let count = self.sync.len().saturating_sub(HEADER_LEN) / REC;
        for i in (0..count).rev() {
            match self.read_record_at(HEADER_LEN + i * REC)? {
                Record::Commit { .. } => continue,
                Record::Data(ev) => {
                    if ev.key_hash == key_hash
                        && ev.key_len as usize == key.len()
                        && self.matches_key(&ev, key)?
                    {
                        return Ok(Some(ev));
                    }
                }
            }
        }
        Ok(None)
    }

    fn read_record_at(&mut self, offset: u64) -> Result<Record, Error> {
        let mut rec = [0u8; SYNC_RECORD_LEN];
        self.sync.read_at(offset, &mut rec)?;
        decode_record(&rec)
    }

    /// Verifies the state record for `ev`, then checks that its key equals
    /// `key`. Separating these lets a hash collision fall through cleanly.
    fn matches_key(&mut self, ev: &ChangeEvent, key: &[u8]) -> Result<bool, Error> {
        self.check_state_record(ev)?;
        let base = ev.payload_offset + STATE_RECORD_HEADER_LEN;
        let mut buf = [0u8; CHUNK];
        let mut pos = 0usize;
        while pos < key.len() {
            let n = min(CHUNK, key.len() - pos);
            self.state.read_at(base + pos as u64, &mut buf[..n])?;
            if buf[..n] != key[pos..pos + n] {
                return Ok(false);
            }
            pos += n;
        }
        Ok(true)
    }

    fn read_value(&mut self, ev: &ChangeEvent, out: &mut [u8]) -> Result<(), Error> {
        let at = ev.payload_offset + STATE_RECORD_HEADER_LEN + u64::from(ev.key_len);
        self.state.read_at(at, out)
    }

    fn check_state_record(&mut self, ev: &ChangeEvent) -> Result<(), Error> {
        if self.state_record_valid(ev)? {
            Ok(())
        } else {
            Err(Error::Corrupt("state record does not match its sync event"))
        }
    }

    /// `true` when the state record `ev` points at is intact: checksum,
    /// lengths, key hash, and bounds all agree with the event.
    fn state_record_valid(&mut self, ev: &ChangeEvent) -> Result<bool, Error> {
        let key_len = u64::from(ev.key_len);
        let value_len = u64::from(ev.payload_len);
        if ev.tombstone && value_len != 0 {
            return Ok(false);
        }
        if ev.payload_offset < HEADER_LEN {
            return Ok(false);
        }
        let total = STATE_RECORD_HEADER_LEN + key_len + value_len;
        match ev.payload_offset.checked_add(total) {
            Some(end) if end <= self.state.len() => {}
            _ => return Ok(false),
        }

        let mut header = [0u8; STATE_RECORD_HEADER_LEN as usize];
        self.state.read_at(ev.payload_offset, &mut header)?;
        if le_u16(&header[4..6]) != ev.key_len || le_u32(&header[6..10]) != ev.payload_len {
            return Ok(false);
        }

        let mut reg = crc32c_update(CRC_INIT, &header[4..10]);
        let mut hash = FNV_OFFSET;
        let key_end = STATE_RECORD_HEADER_LEN + key_len;
        let mut buf = [0u8; CHUNK];
        let mut pos = STATE_RECORD_HEADER_LEN;
        while pos < total {
            let n = min(CHUNK as u64, total - pos) as usize;
            self.state.read_at(ev.payload_offset + pos, &mut buf[..n])?;
            reg = crc32c_update(reg, &buf[..n]);
            if pos < key_end {
                let take = min(n as u64, key_end - pos) as usize;
                hash = fnv1a_update(hash, &buf[..take]);
            }
            pos += n as u64;
        }

        let stored_crc = le_u32(&header[0..4]);
        Ok(crc32c_finish(reg) == stored_crc && hash == ev.key_hash)
    }

    /// Restores a consistent state after open. Returns the clock.
    ///
    /// Pass 1 finds the last valid commit record. Pass 2 checks every record
    /// up to it, and any damage there is an error. Everything after that
    /// record belongs to a batch that never committed, so it is truncated.
    fn recover(&mut self) -> Result<u64, Error> {
        let sync_len = self.sync.len();
        let mut last_commit: Option<u64> = None;
        let mut offset = HEADER_LEN;
        while offset + REC <= sync_len {
            if let Record::Commit { .. } = self.read_record_at(offset)? {
                last_commit = Some(offset);
            }
            offset += REC;
        }
        let Some(commit_at) = last_commit else {
            // No batch ever committed. Only the headers survive.
            self.truncate_to(HEADER_LEN, HEADER_LEN)?;
            return Ok(0);
        };

        let mut valid_state = HEADER_LEN;
        let mut max_lamport = 0u64;
        let mut offset = HEADER_LEN;
        while offset <= commit_at {
            match self.load_record(offset)? {
                None => {
                    return Err(Error::Corrupt(
                        "sync log damaged before its last commit record",
                    ))
                }
                Some(Record::Data(ev)) => {
                    let end = ev.payload_offset
                        + STATE_RECORD_HEADER_LEN
                        + u64::from(ev.key_len)
                        + u64::from(ev.payload_len);
                    valid_state = max(valid_state, end);
                    max_lamport = max(max_lamport, ev.lamport);
                    self.index_add(ev.key_hash, offset);
                }
                Some(Record::Commit { lamport, .. }) => {
                    max_lamport = max(max_lamport, lamport);
                }
            }
            offset += REC;
        }
        self.truncate_to(commit_at + REC, valid_state)?;
        Ok(max_lamport)
    }

    fn truncate_to(&mut self, sync_end: u64, state_end: u64) -> Result<(), Error> {
        let mut dirty = false;
        if self.sync.len() > sync_end {
            self.sync.truncate(sync_end)?;
            dirty = true;
        }
        if self.state.len() > state_end {
            self.state.truncate(state_end)?;
            dirty = true;
        }
        if dirty {
            self.state.flush()?;
            self.sync.flush()?;
        }
        Ok(())
    }

    /// The record at `offset`, if it is intact. Data records also need their
    /// state record to be intact. `None` for any corruption. IO errors still
    /// propagate.
    fn load_record(&mut self, offset: u64) -> Result<Option<Record>, Error> {
        match self.read_record_at(offset) {
            Ok(Record::Commit { lamport, node_id }) => {
                Ok(Some(Record::Commit { lamport, node_id }))
            }
            Ok(Record::Data(ev)) => {
                if self.state_record_valid(&ev)? {
                    Ok(Some(Record::Data(ev)))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }
}

impl<'a, St: Storage, Sy: Storage> Batch<'a, St, Sy> {
    /// Writes made through this batch so far.
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Latest event for `key`, including writes made earlier in this batch.
    pub fn latest_event(&mut self, key: &[u8]) -> Result<Option<ChangeEvent>, Error> {
        check_key(key)?;
        self.store.find(key)
    }

    /// Writes `value` under `key`.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<ChangeEvent, Error> {
        check_key(key)?;
        check_value(value)?;
        let lamport = self
            .store
            .lamport
            .checked_add(1)
            .ok_or(Error::LamportOverflow)?;
        let node_id = self.store.node_id;
        let ev = self
            .store
            .write_pending(key, value, false, lamport, node_id)?;
        self.store.lamport = lamport;
        self.pending += 1;
        Ok(ev)
    }

    /// Deletes `key` if it is live. Returns `false` if it was not.
    pub fn delete(&mut self, key: &[u8]) -> Result<bool, Error> {
        check_key(key)?;
        match self.store.find(key)? {
            Some(ev) if !ev.tombstone => {
                let lamport = self
                    .store
                    .lamport
                    .checked_add(1)
                    .ok_or(Error::LamportOverflow)?;
                let node_id = self.store.node_id;
                self.store.write_pending(key, &[], true, lamport, node_id)?;
                self.store.lamport = lamport;
                self.pending += 1;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Writes a change from another node, keeping its timestamp and origin.
    /// The caller decides that it wins.
    pub fn put_remote(
        &mut self,
        key: &[u8],
        value: &[u8],
        lamport: u64,
        node_id: u128,
        tombstone: bool,
    ) -> Result<ChangeEvent, Error> {
        check_key(key)?;
        check_value(value)?;
        if tombstone && !value.is_empty() {
            return Err(Error::TombstoneWithValue);
        }
        let ev = self
            .store
            .write_pending(key, value, tombstone, lamport, node_id)?;
        self.store.lamport = max(self.store.lamport, lamport);
        self.pending += 1;
        Ok(ev)
    }

    /// Makes the batch durable. Returns the number of writes committed. A
    /// batch with no writes commits nothing.
    pub fn commit(self) -> Result<usize, Error> {
        self.finish(false)
    }

    /// As [`commit`](Self::commit), but always writes a commit record, so the
    /// clock is persisted even when the batch is empty.
    pub fn commit_forced(self) -> Result<usize, Error> {
        self.finish(true)
    }

    fn finish(mut self, force: bool) -> Result<usize, Error> {
        if self.pending == 0 && !force {
            self.committed = true;
            return Ok(0);
        }
        self.store.seal()?;
        self.committed = true;
        Ok(self.pending)
    }
}

impl<St: Storage, Sy: Storage> Drop for Batch<'_, St, Sy> {
    fn drop(&mut self) {
        if self.committed || self.store.failed {
            return;
        }
        if self.store.state.len() == self.start_state && self.store.sync.len() == self.start_sync {
            return;
        }
        let state_ok = self.store.state.truncate(self.start_state).is_ok();
        let sync_ok = self.store.sync.truncate(self.start_sync).is_ok();
        if state_ok && sync_ok {
            self.store.lamport = self.start_lamport;
            self.store.index_retract(self.start_sync);
        } else {
            self.store.failed = true;
        }
    }
}

/// Iterator over write events. Created by [`Hyle::events_since`].
pub struct Events<'a, Sy: Storage> {
    sync: &'a mut Sy,
    offset: u64,
    end: u64,
}

impl<Sy: Storage> Iterator for Events<'_, Sy> {
    type Item = Result<(u64, ChangeEvent), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.offset >= self.end {
                return None;
            }
            let mut rec = [0u8; SYNC_RECORD_LEN];
            if let Err(e) = self.sync.read_at(self.offset, &mut rec) {
                self.offset = self.end;
                return Some(Err(e));
            }
            match decode_record(&rec) {
                Ok(Record::Data(ev)) => {
                    self.offset += REC;
                    return Some(Ok((self.offset, ev)));
                }
                Ok(Record::Commit { .. }) => self.offset += REC,
                Err(e) => {
                    // Stop after the first error so the caller is not handed
                    // further records from a damaged log.
                    self.offset = self.end;
                    return Some(Err(e));
                }
            }
        }
    }
}

/// File layout for a file-backed store named `base`:
///
/// - `<base>.sync`: the Sync Segment. Its header names the generation, and
///   replacing this file is the commit point of compaction.
/// - `<base>.state.<generation>`: the State Segment of that generation.
/// - `<base>.sync.tmp`: scratch space during compaction.
#[cfg(feature = "std")]
impl Hyle<FileStorage, FileStorage> {
    /// Opens a file-backed store. Files are named as in the layout above.
    ///
    /// Before opening, this removes state files from other generations and
    /// any leftover `<base>.sync.tmp`. Those are the remains of a compaction
    /// that did not finish. Only one process may use a store at a time.
    pub fn open_file(base: impl AsRef<std::path::Path>, node_id: u128) -> Result<Self, Error> {
        let base = base.as_ref();
        let generation = current_generation(base)?;
        remove_stale_files(base, generation);
        let state = FileStorage::open(state_path(base, generation))?;
        let sync = FileStorage::open(sync_path(base))?;
        Self::open_with(state, sync, node_id, generation)
    }

    /// Compacts a file-backed store in place and returns the new store.
    ///
    /// The swap runs in this order:
    ///
    /// 1. Write the new State Segment to `<base>.state.<gen+1>`, and the new
    ///    Sync Segment to `<base>.sync.tmp`, then flush both.
    /// 2. Rename `<base>.sync.tmp` over `<base>.sync`. This is the commit
    ///    point.
    /// 3. Delete the old State Segment, best-effort.
    ///
    /// A crash before step 2 leaves the old generation intact. The next open
    /// removes the new state file and the temp file. A crash after step 2
    /// leaves the new generation intact. The next open removes the old state
    /// file.
    #[cfg(feature = "alloc")]
    pub fn compact_file(self, base: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let base = base.as_ref();
        let node_id = self.node_id;
        let old_generation = self.generation;
        let new_generation = old_generation
            .checked_add(1)
            .ok_or(Error::Corrupt("generation counter exhausted"))?;
        let tmp = tmp_sync_path(base);

        let mut old = self;
        {
            let state = FileStorage::create(state_path(base, new_generation))?;
            let sync = FileStorage::create(&tmp)?;
            // Dropping the new store closes its handles. The copy is one
            // batch, and that batch is committed before compact_into returns.
            let _new = old.compact_into(state, sync)?;
        }
        // Release the old handles so the rename can replace the sync file on
        // every platform.
        drop(old);

        std::fs::rename(&tmp, sync_path(base))?;
        remove_stale_files(base, new_generation);
        Self::open_file(base, node_id)
    }
}

/// Reads the node id recorded in a file-backed store's header without
/// opening it for writing or running recovery. Useful for tooling that
/// needs the id before it can call [`Hyle::open_file`].
#[cfg(feature = "std")]
pub fn file_node_id(base: impl AsRef<std::path::Path>) -> Result<u128, Error> {
    let header = read_sync_header(base.as_ref())?;
    check_header(&header, SYNC_MAGIC).map(|(node_id, _)| node_id)
}

#[cfg(feature = "std")]
fn sync_path(base: &std::path::Path) -> std::path::PathBuf {
    append_suffix(base, "sync")
}

#[cfg(feature = "std")]
fn tmp_sync_path(base: &std::path::Path) -> std::path::PathBuf {
    append_suffix(base, "sync.tmp")
}

#[cfg(feature = "std")]
fn state_path(base: &std::path::Path, generation: u32) -> std::path::PathBuf {
    append_suffix(base, &format!("state.{generation}"))
}

#[cfg(feature = "std")]
fn append_suffix(base: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut path = base.as_os_str().to_owned();
    path.push(".");
    path.push(suffix);
    std::path::PathBuf::from(path)
}

/// Reads the 32-byte sync header. A missing or short file reads as all zero
/// bytes, which `check_header` rejects. Callers that accept an empty store
/// must check for that case.
#[cfg(feature = "std")]
fn read_sync_header(base: &std::path::Path) -> Result<[u8; HEADER_LEN as usize], Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(sync_path(base))?;
    let mut header = [0u8; HEADER_LEN as usize];
    file.read_exact(&mut header)?;
    Ok(header)
}

/// Generation of the store at `base`. An absent or empty sync file means a
/// new store, which is generation 0.
#[cfg(feature = "std")]
fn current_generation(base: &std::path::Path) -> Result<u32, Error> {
    use std::io::Read;
    let file = match std::fs::File::open(sync_path(base)) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = Vec::with_capacity(HEADER_LEN as usize);
    file.take(HEADER_LEN).read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    let mut header = [0u8; HEADER_LEN as usize];
    if bytes.len() < header.len() {
        return Err(Error::Corrupt("sync segment shorter than its header"));
    }
    header.copy_from_slice(&bytes);
    check_header(&header, SYNC_MAGIC).map(|(_, generation)| generation)
}

/// Deletes files that belong to other generations, plus any leftover temp
/// sync file. Best-effort: a file that cannot be removed is left for the
/// next open to try again.
#[cfg(feature = "std")]
fn remove_stale_files(base: &std::path::Path, generation: u32) {
    let _ = std::fs::remove_file(tmp_sync_path(base));
    let Some(name) = base.file_name() else {
        return;
    };
    let dir = match base.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => std::path::Path::new("."),
    };
    let prefix = format!("{}.state.", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = file_name.strip_prefix(&prefix) {
            if rest.parse::<u32>().ok() != Some(generation) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn check_key(key: &[u8]) -> Result<(), Error> {
    if key.is_empty() {
        return Err(Error::EmptyKey);
    }
    if key.len() > MAX_KEY_LEN {
        return Err(Error::KeyTooLong);
    }
    Ok(())
}

fn check_value(value: &[u8]) -> Result<(), Error> {
    if u32::try_from(value.len()).is_err() {
        return Err(Error::ValueTooLarge);
    }
    Ok(())
}

/// Writes a fresh header when the segment is empty, or validates an
/// existing one against `node_id`. Returns the segment's generation.
fn init_segment<S: Storage>(
    seg: &mut S,
    magic: [u8; 4],
    node_id: u128,
    fresh_generation: u32,
) -> Result<u32, Error> {
    if seg.is_empty() {
        seg.append(&encode_header(magic, node_id, fresh_generation))?;
        seg.flush()?;
        return Ok(fresh_generation);
    }
    if seg.len() < HEADER_LEN {
        return Err(Error::Corrupt("segment shorter than its header"));
    }
    let mut header = [0u8; HEADER_LEN as usize];
    seg.read_at(0, &mut header)?;
    let (stored, generation) = check_header(&header, magic)?;
    if stored != node_id {
        return Err(Error::NodeIdMismatch {
            stored,
            expected: node_id,
        });
    }
    Ok(generation)
}

/// CRC-32C over a state record's bytes after the CRC field:
/// key_len, value_len, key, value.
fn record_crc(lengths: &[u8], key: &[u8], value: &[u8]) -> u32 {
    let reg = crc32c_update(CRC_INIT, lengths);
    let reg = crc32c_update(reg, key);
    crc32c_finish(crc32c_update(reg, value))
}
