//! The v3 engine: one encrypted, append-only file, batched writes, recovery,
//! and compaction. See `docs/format.md` and ADR 0013.
//!
//! A batch appends sealed data frames without flushing, then appends one commit
//! frame and flushes once. The commit frame carries a count and a SHA-256 digest
//! of the batch's data frames, so a batch is committed only when all of it is
//! on disk. That makes one fsync per commit sufficient.
//!
//! Nonces are a random per-open session prefix followed by a counter that never
//! resets, not even when a batch rolls back. Reusing a nonce under one key would
//! expose the plaintext, so the counter must never go backwards.

use core::cmp::max;

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key as CipherKey, XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};

use ed25519_dalek::{Signer, SigningKey};

use crate::checksum::fnv1a;
use crate::format::{
    decode_data, event_message, ChangeEvent, CommitPayload, DataFields, DataPayload, FrameHeader,
    Header, CHECK_LEN, DATA_FIXED_LEN, EVENT_SIG_LEN, FRAME_HEADER_LEN, HEADER_LEN,
    KEY_CHECK_PLAIN, KIND_COMMIT, KIND_DATA, MAX_KEY_LEN, MAX_SEALED_LEN, TAG_LEN,
};
#[cfg(feature = "std")]
use crate::storage::FileStorage;
use crate::storage::Storage;
use crate::Error;

/// A key and its value, as returned by [`Hyle::live_entries`].
pub type KeyValue = (Vec<u8>, Vec<u8>);

/// Length of the per-open session prefix of every nonce.
const SESSION_LEN: usize = 16;

/// A 32-byte key that seals and opens every frame.
#[derive(Clone, Copy)]
pub struct Key([u8; 32]);

impl Key {
    /// A raw 32-byte key. The caller must keep it secret, and must not reuse it
    /// for unrelated data.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Seed of this node's Ed25519 signing key. It comes from the store key and
    /// the node id, so it is stable across opens and is never stored.
    fn signing_seed(&self, node_id: u128) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"tpt-hyle signing key v1");
        h.update(self.0);
        h.update(node_id.to_le_bytes());
        h.finalize().into()
    }

    /// Derives a key from a passphrase with Argon2id. The salt is stored in the
    /// store's header, so the same passphrase opens the store again.
    #[cfg(feature = "std")]
    pub fn derive(passphrase: &[u8], salt: &[u8; 16]) -> Result<Self, Error> {
        let mut out = [0u8; 32];
        argon2::Argon2::default()
            .hash_password_into(passphrase, salt, &mut out)
            .map_err(|_| Error::Crypto("passphrase derivation failed"))?;
        Ok(Self(out))
    }
}

/// How a store's header salt is treated when it is opened.
#[derive(Clone, Copy)]
enum Salt {
    /// A raw key. A new store gets a zero salt. An existing store's salt is
    /// accepted as it is, since the key check decides.
    Raw,
    /// A passphrase. The header salt must equal this value.
    Given([u8; 16]),
}

/// A sync-native store over one encrypted file.
///
/// Every write is a sealed data frame. A batch makes writes durable together.
/// The store has a *generation*. Compaction increments it, and a peer's sync
/// cursor is valid only for the generation it was taken from.
///
/// Reads take `&mut self`, because they decrypt from the storage. A failed write
/// poisons the store until it is reopened.
pub struct Hyle<S: Storage> {
    storage: S,
    key: Key,
    signer: SigningKey,
    salt: [u8; 16],
    node_id: u128,
    generation: u32,
    session: [u8; SESSION_LEN],
    counter: u64,
    lamport: u64,
    failed: bool,
    /// Frame offsets of data records, per key hash, oldest first. Covers the
    /// committed records and the current batch's pending ones.
    index: BTreeMap<u64, Vec<u64>>,
}

/// A group of writes that commits or disappears together.
///
/// Writes are visible to later calls on the same batch. They become durable
/// with [`Batch::commit`]. Dropping an uncommitted batch truncates the file back
/// to where the batch began.
///
/// ```
/// use tpt_hyle::{Hyle, Key, VecStorage};
///
/// let mut db = Hyle::open_with_session(VecStorage::new(), 1, Key::from_bytes([7; 32]), [1; 16])?;
/// let mut batch = db.batch()?;
/// batch.put(b"a", b"1")?;
/// batch.put(b"b", b"2")?;
/// batch.commit()?; // both writes land, or neither does
/// # Ok::<(), tpt_hyle::Error>(())
/// ```
pub struct Batch<'a, S: Storage> {
    store: &'a mut Hyle<S>,
    start_len: u64,
    start_lamport: u64,
    pending: u32,
    digest: Sha256,
    committed: bool,
}

/// A frame that verified, with the plaintext it carried.
struct Frame {
    kind: u8,
    plain: Vec<u8>,
    raw: Vec<u8>,
    end: u64,
}

#[cfg(feature = "std")]
fn random_bytes<const N: usize>() -> Result<[u8; N], Error> {
    let mut out = [0u8; N];
    getrandom::getrandom(&mut out).map_err(|_| Error::Crypto("no system randomness"))?;
    Ok(out)
}

impl<S: Storage> Hyle<S> {
    /// Opens a store with a raw key. Creates it if the storage is empty.
    ///
    /// Recovery runs here. Frames after the last committed batch are removed.
    /// Damage inside committed data is reported as [`Error::Corrupt`].
    #[cfg(feature = "std")]
    pub fn open(storage: S, node_id: u128, key: Key) -> Result<Self, Error> {
        let session = random_bytes::<SESSION_LEN>()?;
        Self::open_inner(storage, node_id, key, Salt::Raw, 0, session)
    }

    /// Opens a store with a raw key and a caller-supplied session prefix.
    ///
    /// The session must be fresh randomness, different on every open. Two opens
    /// with the same session and key would reuse nonces. Prefer [`Hyle::open`]
    /// where the system has randomness.
    pub fn open_with_session(
        storage: S,
        node_id: u128,
        key: Key,
        session: [u8; SESSION_LEN],
    ) -> Result<Self, Error> {
        Self::open_inner(storage, node_id, key, Salt::Raw, 0, session)
    }

    /// Opens a store with a passphrase. The salt comes from the header of an
    /// existing store, or is created for a new one.
    #[cfg(feature = "std")]
    pub fn open_with_passphrase(
        storage: S,
        node_id: u128,
        passphrase: &[u8],
    ) -> Result<Self, Error> {
        let mut storage = storage;
        let salt = match read_salt(&mut storage)? {
            Some(salt) => salt,
            None => random_bytes::<16>()?,
        };
        let key = Key::derive(passphrase, &salt)?;
        let session = random_bytes::<SESSION_LEN>()?;
        Self::open_inner(storage, node_id, key, Salt::Given(salt), 0, session)
    }

    fn open_inner(
        mut storage: S,
        node_id: u128,
        key: Key,
        salt: Salt,
        fresh_generation: u32,
        session: [u8; SESSION_LEN],
    ) -> Result<Self, Error> {
        let header = if storage.is_empty() {
            let fresh_salt = match salt {
                Salt::Raw => [0u8; 16],
                Salt::Given(s) => s,
            };
            let mut header = Header {
                node_id,
                generation: fresh_generation,
                salt: fresh_salt,
                check_nonce: check_nonce(&session),
                check_sealed: [0u8; CHECK_LEN],
            };
            header.check_sealed = seal_key_check(&key, &header)?;
            storage.append(&header.encode())?;
            storage.flush()?;
            header
        } else {
            if storage.len() < HEADER_LEN {
                return Err(Error::Corrupt("file shorter than its header"));
            }
            let header = read_header(&mut storage)?;
            if header.node_id != node_id {
                return Err(Error::NodeIdMismatch {
                    stored: header.node_id,
                    expected: node_id,
                });
            }
            if let Salt::Given(s) = salt {
                if s != header.salt {
                    return Err(Error::WrongKey);
                }
            }
            if !verify_key(&key, &header) {
                return Err(Error::WrongKey);
            }
            header
        };

        let signer = SigningKey::from_bytes(&key.signing_seed(node_id));
        let mut store = Self {
            storage,
            key,
            signer,
            salt: header.salt,
            node_id,
            generation: header.generation,
            session,
            counter: 0,
            lamport: 0,
            failed: false,
            index: BTreeMap::new(),
        };
        store.lamport = store.recover()?;
        Ok(store)
    }

    /// The node id this store writes under.
    /// This node's Ed25519 public key. Peers check this node's events with it.
    pub fn public_key(&self) -> [u8; 32] {
        self.signer.verifying_key().to_bytes()
    }

    /// The signature stored with an event, which the issuing node made.
    pub fn event_signature(&mut self, ev: &ChangeEvent) -> Result<[u8; EVENT_SIG_LEN], Error> {
        let (_, plain) = self.data_at(ev.payload_offset)?;
        Ok(signature_in(&plain))
    }

    /// Signs an event made by this node.
    fn sign(
        &self,
        lamport: u64,
        node_id: u128,
        tombstone: bool,
        key: &[u8],
        value: &[u8],
    ) -> [u8; EVENT_SIG_LEN] {
        let message = event_message(node_id, lamport, tombstone, key, value);
        self.signer.sign(&message).to_bytes()
    }

    pub fn node_id(&self) -> u128 {
        self.node_id
    }

    /// The generation of this store. Compaction increments it. A peer that
    /// stores a sync cursor must also store the generation it came from. If the
    /// generation changes, the cursor is stale and the peer must resynchronize.
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
    pub fn batch(&mut self) -> Result<Batch<'_, S>, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        Ok(Batch {
            start_len: self.storage.len(),
            start_lamport: self.lamport,
            pending: 0,
            digest: Sha256::new(),
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

    /// Writes a change from another node, keeping its timestamp and origin, in
    /// a batch of one. The caller decides that it wins.
    pub fn put_remote(
        &mut self,
        key: &[u8],
        value: &[u8],
        lamport: u64,
        node_id: u128,
        tombstone: bool,
        signature: [u8; EVENT_SIG_LEN],
    ) -> Result<ChangeEvent, Error> {
        let mut batch = self.batch()?;
        let ev = batch.put_remote(key, value, lamport, node_id, tombstone, signature)?;
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
        let (fields, plain) = self.data_at(ev.payload_offset)?;
        let start = DATA_FIXED_LEN + fields.key_len as usize;
        out[..n].copy_from_slice(&plain[start..start + n]);
        Ok(Some(n))
    }

    /// `true` if `key` holds a live value.
    pub fn contains_key(&mut self, key: &[u8]) -> Result<bool, Error> {
        check_key(key)?;
        Ok(matches!(self.find(key)?, Some(ev) if !ev.tombstone))
    }

    /// Copies the key and value of an event into the caller's buffers. Returns
    /// their lengths.
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
        let (fields, plain) = self.data_at(ev.payload_offset)?;
        if fields.lamport != ev.lamport || fields.node_id != ev.node_id {
            return Err(Error::Corrupt(
                "frame does not match the event that points at it",
            ));
        }
        let vstart = DATA_FIXED_LEN + kl;
        key_out[..kl].copy_from_slice(&plain[DATA_FIXED_LEN..vstart]);
        value_out[..vl].copy_from_slice(&plain[vstart..vstart + vl]);
        Ok((kl, vl))
    }

    /// Iterates the write events in the log from `cursor`. The cursor is a byte
    /// offset: either [`HEADER_LEN`] or the next value of a previous item.
    ///
    /// Each item is `(next_cursor, event)`. Commit frames are skipped. Store
    /// `next_cursor` to resume.
    pub fn events_since(&mut self, cursor: u64) -> Result<Events<'_, S>, Error> {
        let end = self.storage.len();
        if cursor < HEADER_LEN || cursor > end {
            return Err(Error::BadCursor(cursor));
        }
        if cursor < end && self.read_frame(cursor)?.is_none() {
            return Err(Error::BadCursor(cursor));
        }
        Ok(Events {
            store: self,
            offset: cursor,
            end,
        })
    }

    /// End of the log. A peer that has seen everything up to here can use this
    /// as its cursor.
    pub fn sync_end(&self) -> u64 {
        self.storage.len()
    }

    /// The storage the store writes to.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Returns the storage, consuming the store. For reopening and for
    /// fault-injection tests.
    pub fn into_storage(self) -> S {
        self.storage
    }

    /// Rewrites the store into `storage`, which must be empty. The session
    /// prefix must be fresh randomness.
    ///
    /// The new store keeps the latest event for each key, tombstones included.
    /// Each kept event keeps its original Lamport timestamp and node id. The
    /// clock never moves backwards. The new store has the next generation. The
    /// copy is one batch.
    ///
    /// This does not swap files. [`Hyle::compact_file`] does the crash-safe swap.
    pub fn compact_into<S2: Storage>(
        &mut self,
        storage: S2,
        session: [u8; SESSION_LEN],
    ) -> Result<Hyle<S2>, Error> {
        self.compact_into_collecting(storage, session, |_, _| false)
    }

    /// Like [`Hyle::compact_into`], but also drops the latest tombstone of each
    /// key for which `collectable` returns `true`. The closure receives the
    /// tombstone's end offset in this log and its lamport. Dropping a tombstone
    /// lets an older value of the key come back if a stale copy of it is
    /// written later, so only the caller can say when that cannot happen.
    pub fn compact_into_collecting<S2: Storage>(
        &mut self,
        storage: S2,
        session: [u8; SESSION_LEN],
        collectable: impl Fn(u64, u64) -> bool,
    ) -> Result<Hyle<S2>, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        if !storage.is_empty() {
            return Err(Error::TargetNotEmpty);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(Error::Corrupt("generation counter exhausted"))?;

        let events: Vec<(u64, ChangeEvent)> =
            self.events_since(HEADER_LEN)?.collect::<Result<_, _>>()?;

        // Pass 1: each key's last event. Every frame is verified here, so the
        // copy cannot carry damage forward.
        let mut keys: Vec<Vec<u8>> = Vec::with_capacity(events.len());
        let mut latest: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        for (i, (_, ev)) in events.iter().enumerate() {
            let (fields, plain) = self.data_at(ev.payload_offset)?;
            let key = plain[DATA_FIXED_LEN..][..fields.key_len as usize].to_vec();
            latest.insert(key.clone(), i);
            keys.push(key);
        }

        let mut target = Hyle::open_inner(
            storage,
            self.node_id,
            self.key,
            Salt::Given(self.salt),
            generation,
            session,
        )?;
        target.observe_lamport(self.lamport);
        let mut batch = target.batch()?;
        for (i, (end, ev)) in events.iter().enumerate() {
            if latest.get(&keys[i]) != Some(&i) {
                continue;
            }
            if ev.tombstone && collectable(*end, ev.lamport) {
                continue;
            }
            let (fields, plain) = self.data_at(ev.payload_offset)?;
            let value = value_of(&plain, fields.key_len as usize);
            let signature = signature_in(&plain);
            batch.put_remote(
                &keys[i],
                value,
                ev.lamport,
                ev.node_id,
                ev.tombstone,
                signature,
            )?;
        }
        if self.lamport > 0 {
            batch.commit_forced()?;
        } else {
            batch.commit()?;
        }
        Ok(target)
    }

    /// Every live key that starts with `prefix`, with its value, in key order.
    /// Scans the whole log, so the cost is linear in its length.
    pub fn live_entries(&mut self, prefix: &[u8]) -> Result<Vec<KeyValue>, Error> {
        let events: Vec<ChangeEvent> = self
            .events_since(HEADER_LEN)?
            .map(|item| item.map(|(_, ev)| ev))
            .collect::<Result<_, _>>()?;

        let mut latest: BTreeMap<Vec<u8>, Option<ChangeEvent>> = BTreeMap::new();
        for ev in events {
            let (fields, plain) = self.data_at(ev.payload_offset)?;
            let key = plain[DATA_FIXED_LEN..][..fields.key_len as usize].to_vec();
            latest.insert(key, if ev.tombstone { None } else { Some(ev) });
        }

        let mut out = Vec::new();
        for (key, ev) in latest {
            if !key.starts_with(prefix) {
                continue;
            }
            if let Some(ev) = ev {
                let (fields, plain) = self.data_at(ev.payload_offset)?;
                let value = value_of(&plain, fields.key_len as usize).to_vec();
                out.push((key, value));
            }
        }
        Ok(out)
    }

    /// The latest event for `key`, tombstones included. Replication uses it to
    /// decide whether an incoming write wins.
    pub fn latest_event(&mut self, key: &[u8]) -> Result<Option<ChangeEvent>, Error> {
        check_key(key)?;
        self.find(key)
    }

    /// The value for `key` in a new vector. `None` if absent or deleted.
    #[cfg(feature = "json")]
    pub(crate) fn get_vec(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        check_key(key)?;
        let ev = match self.find(key)? {
            Some(ev) if !ev.tombstone => ev,
            _ => return Ok(None),
        };
        let (fields, plain) = self.data_at(ev.payload_offset)?;
        Ok(Some(value_of(&plain, fields.key_len as usize).to_vec()))
    }

    /// Seals a plaintext as a frame of `kind`. The nonce is the session prefix
    /// and the counter. The counter advances for every frame and is never
    /// rewound, so a nonce is never reused under this key.
    fn seal(&mut self, kind: u8, plain: &[u8]) -> Result<Vec<u8>, Error> {
        if plain.len() + TAG_LEN > MAX_SEALED_LEN as usize {
            return Err(Error::ValueTooLarge);
        }
        let counter = self.counter;
        self.counter = counter
            .checked_add(1)
            .ok_or(Error::Corrupt("nonce counter exhausted"))?;
        let mut nonce = [0u8; 24];
        nonce[..SESSION_LEN].copy_from_slice(&self.session);
        nonce[SESSION_LEN..].copy_from_slice(&counter.to_le_bytes());
        let frame_header = FrameHeader {
            sealed_len: (plain.len() + TAG_LEN) as u32,
            kind,
            nonce,
        };
        let aad = frame_header.aad();
        let sealed = self
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plain,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Crypto("seal failed"))?;
        let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + sealed.len());
        frame.extend_from_slice(&frame_header.encode());
        frame.extend_from_slice(&sealed);
        Ok(frame)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(CipherKey::from_slice(&self.key.0))
    }

    /// Reads and authenticates the frame at `offset`. `None` if there is no
    /// valid frame there.
    fn read_frame(&mut self, offset: u64) -> Result<Option<Frame>, Error> {
        let len = self.storage.len();
        if offset + FRAME_HEADER_LEN as u64 > len {
            return Ok(None);
        }
        let mut hb = [0u8; FRAME_HEADER_LEN];
        self.storage.read_at(offset, &mut hb)?;
        let Some(fh) = FrameHeader::decode(&hb) else {
            return Ok(None);
        };
        let total = FRAME_HEADER_LEN as u64 + u64::from(fh.sealed_len);
        if offset + total > len {
            return Ok(None);
        }
        let mut raw = vec![0u8; total as usize];
        self.storage.read_at(offset, &mut raw)?;
        let aad = fh.aad();
        let plain = match self.cipher().decrypt(
            XNonce::from_slice(&fh.nonce),
            Payload {
                msg: &raw[FRAME_HEADER_LEN..],
                aad: &aad,
            },
        ) {
            Ok(plain) => plain,
            Err(_) => return Ok(None),
        };
        Ok(Some(Frame {
            kind: fh.kind,
            plain,
            raw,
            end: offset + total,
        }))
    }

    /// The data frame at `offset`, verified, with its parsed fields and its
    /// plaintext.
    fn data_at(&mut self, offset: u64) -> Result<(DataFields, Vec<u8>), Error> {
        let frame = self
            .read_frame(offset)?
            .ok_or(Error::Corrupt("frame does not verify"))?;
        if frame.kind != KIND_DATA {
            return Err(Error::Corrupt("expected a data frame"));
        }
        let fields = decode_data(&frame.plain).ok_or(Error::Corrupt("malformed data frame"))?;
        Ok((fields, frame.plain))
    }

    /// The latest event on `key`, through the index. Each candidate is checked
    /// against the full key, so a hash collision cannot return the wrong value.
    fn find(&mut self, key: &[u8]) -> Result<Option<ChangeEvent>, Error> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        let key_hash = fnv1a(key);
        let count = self.index.get(&key_hash).map_or(0, Vec::len);
        for i in (0..count).rev() {
            let Some(&offset) = self.index.get(&key_hash).and_then(|v| v.get(i)) else {
                break;
            };
            let (fields, plain) = self.data_at(offset)?;
            let k = &plain[DATA_FIXED_LEN..][..fields.key_len as usize];
            if k == key {
                return Ok(Some(ChangeEvent {
                    key_hash,
                    key_len: fields.key_len,
                    lamport: fields.lamport,
                    node_id: fields.node_id,
                    payload_offset: offset,
                    payload_len: fields.value_len,
                    tombstone: fields.tombstone,
                }));
            }
        }
        Ok(None)
    }

    fn index_add(&mut self, key_hash: u64, offset: u64) {
        self.index.entry(key_hash).or_default().push(offset);
    }

    /// Forgets index entries at or after `from`. Used when a batch rolls back.
    fn index_retract(&mut self, from: u64) {
        self.index.retain(|_, offsets| {
            offsets.retain(|&o| o < from);
            !offsets.is_empty()
        });
    }

    /// Makes a batch durable: one commit frame, then one flush.
    fn seal_commit(&mut self, digest: [u8; 32], count: u32) -> Result<(), Error> {
        let payload = CommitPayload {
            lamport: self.lamport,
            node_id: self.node_id,
            count,
            digest,
        }
        .encode();
        let frame = self.seal(KIND_COMMIT, &payload)?;
        self.storage.append(&frame)?;
        self.storage.flush()?;
        Ok(())
    }

    /// Scans the log on open. Returns the clock.
    ///
    /// Frames are read in order. A data frame joins the current batch. A commit
    /// frame commits the batch when its count and digest match. Frames after the
    /// last commit belong to a batch that never committed, and are truncated.
    ///
    /// A frame that fails to verify ends the scan. If a later frame still
    /// verifies, the damage is inside committed data, so open fails rather than
    /// drop it.
    fn recover(&mut self) -> Result<u64, Error> {
        let mut offset = HEADER_LEN;
        let mut committed_end = HEADER_LEN;
        let mut max_lamport = 0u64;
        let mut digest = Sha256::new();
        let mut batch_count = 0u32;
        let mut batch_lamport = 0u64;
        let mut batch: Vec<(u64, u64)> = Vec::new();
        let mut committed: Vec<(u64, u64)> = Vec::new();

        loop {
            let Some(frame) = self.read_frame(offset)? else {
                if committed_end < self.storage.len() && self.frame_after(offset)? {
                    return Err(Error::Corrupt("damage inside committed data"));
                }
                break;
            };
            match frame.kind {
                KIND_DATA => {
                    let Some(fields) = decode_data(&frame.plain) else {
                        break;
                    };
                    digest.update(&frame.raw);
                    batch_count = batch_count
                        .checked_add(1)
                        .ok_or(Error::Corrupt("batch too large"))?;
                    batch_lamport = max(batch_lamport, fields.lamport);
                    let key = &frame.plain[DATA_FIXED_LEN..][..fields.key_len as usize];
                    batch.push((fnv1a(key), offset));
                }
                KIND_COMMIT => {
                    let Some(commit) = CommitPayload::decode(&frame.plain) else {
                        break;
                    };
                    let actual: [u8; 32] = digest.finalize_reset().into();
                    if commit.count != batch_count || commit.digest != actual {
                        return Err(Error::Corrupt("commit does not match its batch"));
                    }
                    max_lamport = max(max_lamport, max(commit.lamport, batch_lamport));
                    committed.append(&mut batch);
                    committed_end = frame.end;
                    batch_count = 0;
                    batch_lamport = 0;
                }
                _ => break,
            }
            offset = frame.end;
        }

        if self.storage.len() > committed_end {
            self.storage.truncate(committed_end)?;
            self.storage.flush()?;
        }
        for (key_hash, at) in committed {
            self.index_add(key_hash, at);
        }
        Ok(max_lamport)
    }

    /// `true` if a valid frame follows a broken one at `offset`. This tells
    /// damage inside committed data apart from an incomplete last batch.
    fn frame_after(&mut self, offset: u64) -> Result<bool, Error> {
        if offset + FRAME_HEADER_LEN as u64 > self.storage.len() {
            return Ok(false);
        }
        let mut hb = [0u8; FRAME_HEADER_LEN];
        self.storage.read_at(offset, &mut hb)?;
        let sealed_len = crate::format::le_u32(&hb[0..4]);
        if sealed_len < TAG_LEN as u32 || sealed_len > MAX_SEALED_LEN {
            return Ok(false);
        }
        let next = offset + FRAME_HEADER_LEN as u64 + u64::from(sealed_len);
        Ok(self.read_frame(next)?.is_some())
    }
}

/// Iterator over write events. Created by [`Hyle::events_since`].
pub struct Events<'a, S: Storage> {
    store: &'a mut Hyle<S>,
    offset: u64,
    end: u64,
}

impl<S: Storage> Iterator for Events<'_, S> {
    type Item = Result<(u64, ChangeEvent), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.offset >= self.end {
                return None;
            }
            let frame = match self.store.read_frame(self.offset) {
                Ok(Some(frame)) => frame,
                Ok(None) => {
                    self.offset = self.end;
                    return Some(Err(Error::Corrupt("frame does not verify")));
                }
                Err(e) => {
                    self.offset = self.end;
                    return Some(Err(e));
                }
            };
            let at = self.offset;
            self.offset = frame.end;
            if frame.kind != KIND_DATA {
                continue;
            }
            let Some(fields) = decode_data(&frame.plain) else {
                self.offset = self.end;
                return Some(Err(Error::Corrupt("malformed data frame")));
            };
            let key = &frame.plain[DATA_FIXED_LEN..][..fields.key_len as usize];
            let ev = ChangeEvent {
                key_hash: fnv1a(key),
                key_len: fields.key_len,
                lamport: fields.lamport,
                node_id: fields.node_id,
                payload_offset: at,
                payload_len: fields.value_len,
                tombstone: fields.tombstone,
            };
            return Some(Ok((self.offset, ev)));
        }
    }
}

impl<S: Storage> Batch<'_, S> {
    /// Writes made through this batch so far.
    pub fn pending(&self) -> u32 {
        self.pending
    }

    /// The latest event for `key`, including writes made earlier in this batch.
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
        let ev = self.write(key, value, false, lamport, node_id, None)?;
        self.store.lamport = lamport;
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
                self.write(key, &[], true, lamport, node_id, None)?;
                self.store.lamport = lamport;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Writes a change from another node, keeping its timestamp and origin. The
    /// caller decides that it wins.
    pub fn put_remote(
        &mut self,
        key: &[u8],
        value: &[u8],
        lamport: u64,
        node_id: u128,
        tombstone: bool,
        signature: [u8; EVENT_SIG_LEN],
    ) -> Result<ChangeEvent, Error> {
        check_key(key)?;
        check_value(value)?;
        if tombstone && !value.is_empty() {
            return Err(Error::TombstoneWithValue);
        }
        let ev = self.write(key, value, tombstone, lamport, node_id, Some(signature))?;
        self.store.lamport = max(self.store.lamport, lamport);
        Ok(ev)
    }

    fn write(
        &mut self,
        key: &[u8],
        value: &[u8],
        tombstone: bool,
        lamport: u64,
        node_id: u128,
        signature: Option<[u8; EVENT_SIG_LEN]>,
    ) -> Result<ChangeEvent, Error> {
        if self.store.failed {
            return Err(Error::Poisoned);
        }
        let signature = match signature {
            Some(sig) => sig,
            None => self.store.sign(lamport, node_id, tombstone, key, value),
        };
        let plain = DataPayload {
            lamport,
            node_id,
            tombstone,
            key,
            value,
            signature,
        }
        .encode();
        let frame = self.store.seal(KIND_DATA, &plain)?;
        let offset = match self.store.storage.append(&frame) {
            Ok(offset) => offset,
            Err(e) => {
                self.store.failed = true;
                return Err(e);
            }
        };
        self.digest.update(&frame);
        self.pending = self
            .pending
            .checked_add(1)
            .ok_or(Error::Corrupt("batch too large"))?;
        let key_hash = fnv1a(key);
        self.store.index_add(key_hash, offset);
        Ok(ChangeEvent {
            key_hash,
            key_len: key.len() as u16,
            lamport,
            node_id,
            payload_offset: offset,
            payload_len: value.len() as u32,
            tombstone,
        })
    }

    /// Makes the batch durable. Returns the number of writes committed. A batch
    /// with no writes commits nothing.
    pub fn commit(self) -> Result<u32, Error> {
        self.finish(false)
    }

    /// As [`commit`](Self::commit), but always writes a commit frame, so the
    /// clock is persisted even when the batch is empty.
    pub fn commit_forced(self) -> Result<u32, Error> {
        self.finish(true)
    }

    fn finish(mut self, force: bool) -> Result<u32, Error> {
        if self.pending == 0 && !force {
            self.committed = true;
            return Ok(0);
        }
        let digest: [u8; 32] = self.digest.clone().finalize().into();
        match self.store.seal_commit(digest, self.pending) {
            Ok(()) => {
                self.committed = true;
                Ok(self.pending)
            }
            Err(e) => {
                self.store.failed = true;
                Err(e)
            }
        }
    }
}

impl<S: Storage> Drop for Batch<'_, S> {
    fn drop(&mut self) {
        if self.committed || self.store.failed {
            return;
        }
        if self.store.storage.len() == self.start_len {
            return;
        }
        match self.store.storage.truncate(self.start_len) {
            Ok(()) => {
                self.store.lamport = self.start_lamport;
                self.store.index_retract(self.start_len);
            }
            Err(_) => self.store.failed = true,
        }
    }
}

#[cfg(feature = "std")]
impl Hyle<FileStorage> {
    /// Opens a file-backed store with a raw key. Creates the file if needed.
    pub fn open_file_with_key(
        path: impl AsRef<std::path::Path>,
        node_id: u128,
        key: Key,
    ) -> Result<Self, Error> {
        let storage = FileStorage::open(path)?;
        Self::open(storage, node_id, key)
    }

    /// Opens a file-backed store with a passphrase. Creates the file if needed.
    pub fn open_file(
        path: impl AsRef<std::path::Path>,
        node_id: u128,
        passphrase: &[u8],
    ) -> Result<Self, Error> {
        let storage = FileStorage::open(path)?;
        Self::open_with_passphrase(storage, node_id, passphrase)
    }

    /// Compacts a file-backed store in place and returns the new store.
    ///
    /// 1. Write the compacted store to `<path>.tmp` and flush it.
    /// 2. Rename `<path>.tmp` over `<path>`. This is the commit point.
    /// 3. Reopen.
    ///
    /// A crash before step 2 leaves the old file intact, and the leftover
    /// `<path>.tmp` is removed on the next compaction. A rename is atomic, so a
    /// crash after it leaves the new file.
    pub fn compact_file(self, path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        self.compact_file_collecting(path, |_, _| false)
    }

    /// File version of [`Hyle::compact_into_collecting`]. Writes to a temporary
    /// file and renames it over `path`, so a crash leaves one complete file.
    pub fn compact_file_collecting(
        self,
        path: impl AsRef<std::path::Path>,
        collectable: impl Fn(u64, u64) -> bool,
    ) -> Result<Self, Error> {
        let path = path.as_ref();
        let node_id = self.node_id;
        let key = self.key;
        let tmp = tmp_path(path);
        let session = random_bytes::<SESSION_LEN>()?;

        let mut old = self;
        {
            let storage = FileStorage::create(&tmp)?;
            let _new = old.compact_into_collecting(storage, session, collectable)?;
        }
        drop(old);

        std::fs::rename(&tmp, path)?;
        Self::open_file_with_key(path, node_id, key)
    }
}

/// Reads the node id of a file-backed store without opening it for writing, so
/// tooling can see the id before it has the key.
#[cfg(feature = "std")]
pub fn file_node_id(path: impl AsRef<std::path::Path>) -> Result<u128, Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut bytes = [0u8; HEADER_LEN as usize];
    file.read_exact(&mut bytes)?;
    Header::decode(&bytes)
        .map(|h| h.node_id)
        .map_err(|_| Error::Corrupt("not a tpt-hyle store"))
}

#[cfg(feature = "std")]
fn tmp_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    std::path::PathBuf::from(name)
}

/// The nonce for a header's key-check value. It uses the session prefix with
/// the largest counter value, which no frame reaches, so it never collides with
/// a frame nonce under the same key.
fn check_nonce(session: &[u8; SESSION_LEN]) -> [u8; 24] {
    let mut nonce = [0u8; 24];
    nonce[..SESSION_LEN].copy_from_slice(session);
    nonce[SESSION_LEN..].copy_from_slice(&u64::MAX.to_le_bytes());
    nonce
}

/// Seals the key-check value into a header. The header's fixed fields are the
/// associated data, so changing them later fails the check.
fn seal_key_check(key: &Key, header: &Header) -> Result<[u8; CHECK_LEN], Error> {
    let cipher = XChaCha20Poly1305::new(CipherKey::from_slice(&key.0));
    let aad = header.authenticated_prefix();
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&header.check_nonce),
            Payload {
                msg: KEY_CHECK_PLAIN,
                aad: &aad,
            },
        )
        .map_err(|_| Error::Crypto("key check failed"))?;
    let mut out = [0u8; CHECK_LEN];
    out.copy_from_slice(&sealed);
    Ok(out)
}

/// `true` if `key` opens the header's key-check value.
fn verify_key(key: &Key, header: &Header) -> bool {
    let cipher = XChaCha20Poly1305::new(CipherKey::from_slice(&key.0));
    let aad = header.authenticated_prefix();
    match cipher.decrypt(
        XNonce::from_slice(&header.check_nonce),
        Payload {
            msg: &header.check_sealed,
            aad: &aad,
        },
    ) {
        Ok(plain) => plain == KEY_CHECK_PLAIN,
        Err(_) => false,
    }
}

fn read_header<S: Storage>(storage: &mut S) -> Result<Header, Error> {
    let mut bytes = [0u8; HEADER_LEN as usize];
    storage.read_at(0, &mut bytes)?;
    Header::decode(&bytes).map_err(|_| Error::Corrupt("not a tpt-hyle store"))
}

/// The header salt of an existing store, or `None` for an empty one.
#[cfg(feature = "std")]
fn read_salt<S: Storage>(storage: &mut S) -> Result<Option<[u8; 16]>, Error> {
    if storage.len() < HEADER_LEN {
        return Ok(None);
    }
    Ok(Some(read_header(storage)?.salt))
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

/// The value inside a data plaintext, without the signature that follows it.
fn value_of(plain: &[u8], key_len: usize) -> &[u8] {
    &plain[DATA_FIXED_LEN + key_len..plain.len() - EVENT_SIG_LEN]
}

/// The signature at the end of a data plaintext.
fn signature_in(plain: &[u8]) -> [u8; EVENT_SIG_LEN] {
    let mut sig = [0u8; EVENT_SIG_LEN];
    sig.copy_from_slice(&plain[plain.len() - EVENT_SIG_LEN..]);
    sig
}
