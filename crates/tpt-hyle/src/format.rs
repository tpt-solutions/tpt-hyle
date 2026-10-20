//! On-disk layout, version 3. The authoritative description is
//! `docs/format.md`. Keep the two in sync.
//!
//! A store is one file: a 128-byte header, then frames. Every frame is sealed
//! with XChaCha20-Poly1305. A data frame carries one write. A commit frame
//! closes a batch, and carries a count and a SHA-256 digest of the batch's
//! data frames. A batch is committed only when its commit frame verifies.
//!
//! All integers are little-endian.

use core::fmt;

/// Version written into every header.
pub const FORMAT_VERSION: u16 = 3;
/// Magic bytes at the start of every store file.
pub const MAGIC: [u8; 4] = *b"HYLG";
/// Length of the header. Frames start here.
pub const HEADER_LEN: u64 = 128;
/// Length of a frame header: length(4), kind(1), nonce(24).
pub const FRAME_HEADER_LEN: usize = 29;
/// Length of the Poly1305 tag that ends every sealed payload.
pub const TAG_LEN: usize = 16;
/// Largest sealed payload accepted. A guard against hostile lengths.
pub const MAX_SEALED_LEN: u32 = 1 << 26;
/// Longest key the format can express.
pub const MAX_KEY_LEN: usize = u16::MAX as usize;
/// Plaintext checked by the header's key-check value.
pub const KEY_CHECK_PLAIN: &[u8] = b"tpt-hyle key check";

/// Frame kind: one write or delete.
pub const KIND_DATA: u8 = 1;
/// Frame kind: closes a batch.
pub const KIND_COMMIT: u8 = 2;

/// Fixed part of a data payload: lamport(8), node(16), flags(1), key_len(2),
/// value_len(4).
pub const DATA_FIXED_LEN: usize = 31;
/// Flag bit in a data payload marking a deletion.
pub const FLAG_TOMBSTONE: u8 = 0b0000_0001;
/// Commit payload: lamport(8), node(16), count(4), digest(32).
pub const COMMIT_PAYLOAD_LEN: usize = 60;

/// The parts of a header that the store reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Node id the store was created under.
    pub node_id: u128,
    /// Generation, incremented by compaction.
    pub generation: u32,
    /// Salt for passphrase derivation. All zero for a raw key.
    pub salt: [u8; 16],
    /// Nonce used to seal the key-check value.
    pub check_nonce: [u8; 24],
    /// Sealed [`KEY_CHECK_PLAIN`].
    pub check_sealed: [u8; KEY_CHECK_PLAIN.len() + TAG_LEN],
}

/// Serialized header length, with the key-check value laid out in it.
const CHECK_OFFSET: usize = 68;

impl Header {
    /// The bytes covered by the key-check value's associated data: everything
    /// before the check value, so a changed node id or generation is detected.
    pub(crate) fn authenticated_prefix(&self) -> [u8; CHECK_OFFSET] {
        let mut out = [0u8; CHECK_OFFSET];
        out[0..4].copy_from_slice(&MAGIC);
        out[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        out[8..24].copy_from_slice(&self.node_id.to_le_bytes());
        out[24..28].copy_from_slice(&self.generation.to_le_bytes());
        out[28..44].copy_from_slice(&self.salt);
        out[44..68].copy_from_slice(&self.check_nonce);
        out
    }

    /// Serializes the header to its 128 bytes.
    pub fn encode(&self) -> [u8; HEADER_LEN as usize] {
        let mut out = [0u8; HEADER_LEN as usize];
        out[..CHECK_OFFSET].copy_from_slice(&self.authenticated_prefix());
        out[CHECK_OFFSET..CHECK_OFFSET + self.check_sealed.len()]
            .copy_from_slice(&self.check_sealed);
        out
    }

    /// Parses the fixed fields. Key-check verification happens in the store,
    /// because it needs the key.
    pub fn decode(bytes: &[u8; HEADER_LEN as usize]) -> Result<Self, HeaderError> {
        if bytes[0..4] != MAGIC {
            return Err(HeaderError::BadMagic);
        }
        if le_u16(&bytes[4..6]) != FORMAT_VERSION {
            return Err(HeaderError::BadVersion);
        }
        if bytes[6..8] != [0, 0]
            || bytes[CHECK_OFFSET + KEY_CHECK_PLAIN.len() + TAG_LEN..]
                .iter()
                .any(|&b| b != 0)
        {
            return Err(HeaderError::Reserved);
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&bytes[28..44]);
        let mut check_nonce = [0u8; 24];
        check_nonce.copy_from_slice(&bytes[44..68]);
        let mut check_sealed = [0u8; KEY_CHECK_PLAIN.len() + TAG_LEN];
        check_sealed.copy_from_slice(&bytes[CHECK_OFFSET..CHECK_OFFSET + CHECK_LEN]);
        Ok(Self {
            node_id: le_u128(&bytes[8..24]),
            generation: le_u32(&bytes[24..28]),
            salt,
            check_nonce,
            check_sealed,
        })
    }
}

/// Why a header failed to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    /// Not a store file.
    BadMagic,
    /// Written by a different format version.
    BadVersion,
    /// Reserved bytes are not zero.
    Reserved,
}

/// Bytes of the key-check value, for the header.
pub const CHECK_LEN: usize = KEY_CHECK_PLAIN.len() + TAG_LEN;

/// Frame header fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Length of the sealed payload that follows, tag included.
    pub sealed_len: u32,
    /// `KIND_DATA` or `KIND_COMMIT`.
    pub kind: u8,
    /// The nonce the payload was sealed with.
    pub nonce: [u8; 24],
}

impl FrameHeader {
    /// Serializes the frame header.
    pub fn encode(&self) -> [u8; FRAME_HEADER_LEN] {
        let mut out = [0u8; FRAME_HEADER_LEN];
        out[0..4].copy_from_slice(&self.sealed_len.to_le_bytes());
        out[4] = self.kind;
        out[5..29].copy_from_slice(&self.nonce);
        out
    }

    /// Parses a frame header. Length bounds are checked here, before anything
    /// is read or allocated.
    pub fn decode(bytes: &[u8; FRAME_HEADER_LEN]) -> Option<Self> {
        let sealed_len = le_u32(&bytes[0..4]);
        if sealed_len < TAG_LEN as u32 || sealed_len > MAX_SEALED_LEN {
            return None;
        }
        let kind = bytes[4];
        if kind != KIND_DATA && kind != KIND_COMMIT {
            return None;
        }
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&bytes[5..29]);
        Some(Self {
            sealed_len,
            kind,
            nonce,
        })
    }

    /// The associated data a payload is sealed with. It binds the kind and the
    /// length, so neither can be changed without failing authentication.
    pub fn aad(&self) -> [u8; 5] {
        let mut aad = [0u8; 5];
        aad[0] = self.kind;
        aad[1..5].copy_from_slice(&self.sealed_len.to_le_bytes());
        aad
    }
}

/// A write or delete, as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeEvent {
    /// FNV-1a 64 hash of the key. A lookup filter only.
    pub key_hash: u64,
    /// Length of the key in bytes.
    pub key_len: u16,
    /// Lamport timestamp of the write.
    pub lamport: u64,
    /// Node that issued the write.
    pub node_id: u128,
    /// Byte offset of the data frame in the store file.
    pub payload_offset: u64,
    /// Length of the value in bytes. Zero for tombstones.
    pub payload_len: u32,
    /// `true` if this event deletes the key.
    pub tombstone: bool,
}

/// The plaintext of a data frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPayload<'a> {
    /// Lamport timestamp.
    pub lamport: u64,
    /// Issuing node.
    pub node_id: u128,
    /// Deletion flag.
    pub tombstone: bool,
    /// Key bytes.
    pub key: &'a [u8],
    /// Value bytes.
    pub value: &'a [u8],
}

#[cfg(feature = "alloc")]
impl DataPayload<'_> {
    /// Serializes the payload.
    pub fn encode(&self) -> alloc::vec::Vec<u8> {
        let mut out =
            alloc::vec::Vec::with_capacity(DATA_FIXED_LEN + self.key.len() + self.value.len());
        out.extend_from_slice(&self.lamport.to_le_bytes());
        out.extend_from_slice(&self.node_id.to_le_bytes());
        out.push(if self.tombstone { FLAG_TOMBSTONE } else { 0 });
        out.extend_from_slice(&(self.key.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.value.len() as u32).to_le_bytes());
        out.extend_from_slice(self.key);
        out.extend_from_slice(self.value);
        out
    }
}

/// Parsed view of a data plaintext: where the key and value sit inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataFields {
    /// Lamport timestamp.
    pub lamport: u64,
    /// Issuing node.
    pub node_id: u128,
    /// Deletion flag.
    pub tombstone: bool,
    /// Key length.
    pub key_len: u16,
    /// Value length.
    pub value_len: u32,
}

/// Parses a data plaintext. Rejects lengths that do not add up exactly.
pub fn decode_data(plain: &[u8]) -> Option<DataFields> {
    if plain.len() < DATA_FIXED_LEN {
        return None;
    }
    let flags = plain[24];
    if flags & !FLAG_TOMBSTONE != 0 {
        return None;
    }
    let key_len = le_u16(&plain[25..27]);
    let value_len = le_u32(&plain[27..31]);
    if plain.len() != DATA_FIXED_LEN + key_len as usize + value_len as usize {
        return None;
    }
    if key_len == 0 {
        return None;
    }
    Some(DataFields {
        lamport: le_u64(&plain[0..8]),
        node_id: le_u128(&plain[8..24]),
        tombstone: flags & FLAG_TOMBSTONE != 0,
        key_len,
        value_len,
    })
}

/// The plaintext of a commit frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitPayload {
    /// The clock when the batch committed.
    pub lamport: u64,
    /// The node that committed.
    pub node_id: u128,
    /// Number of data frames in the batch.
    pub count: u32,
    /// SHA-256 over the raw bytes of those data frames, in order.
    pub digest: [u8; 32],
}

impl CommitPayload {
    /// Serializes the payload.
    pub fn encode(&self) -> [u8; COMMIT_PAYLOAD_LEN] {
        let mut out = [0u8; COMMIT_PAYLOAD_LEN];
        out[0..8].copy_from_slice(&self.lamport.to_le_bytes());
        out[8..24].copy_from_slice(&self.node_id.to_le_bytes());
        out[24..28].copy_from_slice(&self.count.to_le_bytes());
        out[28..60].copy_from_slice(&self.digest);
        out
    }

    /// Parses a commit plaintext of exactly the right length.
    pub fn decode(plain: &[u8]) -> Option<Self> {
        if plain.len() != COMMIT_PAYLOAD_LEN {
            return None;
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&plain[28..60]);
        Some(Self {
            lamport: le_u64(&plain[0..8]),
            node_id: le_u128(&plain[8..24]),
            count: le_u32(&plain[24..28]),
            digest,
        })
    }
}

impl fmt::Display for ChangeEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lamport {} from node {:#x}", self.lamport, self.node_id)
    }
}

pub(crate) fn le_u16(b: &[u8]) -> u16 {
    let mut a = [0u8; 2];
    a.copy_from_slice(b);
    u16::from_le_bytes(a)
}

pub(crate) fn le_u32(b: &[u8]) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(b);
    u32::from_le_bytes(a)
}

pub(crate) fn le_u64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    u64::from_le_bytes(a)
}

pub(crate) fn le_u128(b: &[u8]) -> u128 {
    let mut a = [0u8; 16];
    a.copy_from_slice(b);
    u128::from_le_bytes(a)
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every possible header is parsed or rejected. None panics.
    #[kani::proof]
    fn header_decode_never_panics() {
        let bytes: [u8; HEADER_LEN as usize] = kani::any();
        let _ = Header::decode(&bytes);
    }

    /// Every possible frame header is parsed or rejected, and a parsed length
    /// is always within the guard.
    #[kani::proof]
    fn frame_header_decode_is_bounded() {
        let bytes: [u8; FRAME_HEADER_LEN] = kani::any();
        if let Some(h) = FrameHeader::decode(&bytes) {
            assert!(h.sealed_len >= TAG_LEN as u32);
            assert!(h.sealed_len <= MAX_SEALED_LEN);
        }
    }

    /// Every possible data plaintext of a fixed size is parsed or rejected.
    /// None panics, and an accepted one has lengths that add up exactly.
    #[kani::proof]
    fn data_decode_never_panics() {
        let bytes: [u8; 40] = kani::any();
        if let Some(f) = decode_data(&bytes) {
            assert_eq!(
                bytes.len(),
                DATA_FIXED_LEN + f.key_len as usize + f.value_len as usize
            );
        }
    }

    /// Every possible commit plaintext of the right length is parsed.
    #[kani::proof]
    fn commit_decode_never_panics() {
        let bytes: [u8; COMMIT_PAYLOAD_LEN] = kani::any();
        assert!(CommitPayload::decode(&bytes).is_some());
    }
}
