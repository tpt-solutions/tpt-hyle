//! On-disk layout: segment headers, state records and sync records.
//! The authoritative description is `docs/format.md`. Keep the two in sync.
//!
//! All integers are little-endian.

use crate::checksum::crc32c;
use crate::Error;

/// Version written into every segment header.
pub const FORMAT_VERSION: u16 = 2;
/// Length of every segment header. Record offsets start here.
pub const HEADER_LEN: u64 = 32;
/// Magic bytes of the State Segment.
pub const STATE_MAGIC: [u8; 4] = *b"HYST";
/// Magic bytes of the Sync Segment.
pub const SYNC_MAGIC: [u8; 4] = *b"HYSY";
/// Fixed prefix of a state record: crc32c, key_len, value_len.
pub const STATE_RECORD_HEADER_LEN: u64 = 10;
/// Fixed size of every sync record.
pub const SYNC_RECORD_LEN: usize = 56;
/// Longest key the format can express (`key_len` is a `u16`).
pub const MAX_KEY_LEN: usize = u16::MAX as usize;

/// Flag bit in a sync record marking a deletion.
const FLAG_TOMBSTONE: u8 = 0b0000_0001;
/// Flag bit marking a commit record. It closes a batch, and nothing before
/// the last valid commit record is ever discarded on open.
const FLAG_COMMIT: u8 = 0b0000_0010;

/// One sync-log record of either kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Record {
    /// A write or delete.
    Data(ChangeEvent),
    /// Closes a batch. Carries the clock at commit time, so the clock
    /// survives a restart even when the batch held no events.
    Commit { lamport: u64, node_id: u128 },
}

/// Serializes a commit record. Layout: crc32c(4), zeros(10), lamport(8),
/// node_id(16), zeros(14), flags(1) = COMMIT, zeros(3).
pub(crate) fn encode_commit(lamport: u64, node_id: u128) -> [u8; SYNC_RECORD_LEN] {
    let mut rec = [0u8; SYNC_RECORD_LEN];
    rec[12..20].copy_from_slice(&lamport.to_le_bytes());
    rec[20..36].copy_from_slice(&node_id.to_le_bytes());
    rec[50] = FLAG_COMMIT;
    let crc = crc32c(&rec[4..]);
    rec[0..4].copy_from_slice(&crc.to_le_bytes());
    rec
}

/// Parses and checksum-verifies any sync record.
pub(crate) fn decode_record(rec: &[u8; SYNC_RECORD_LEN]) -> Result<Record, Error> {
    if le_u32(&rec[0..4]) != crc32c(&rec[4..]) {
        return Err(Error::Corrupt("sync record checksum mismatch"));
    }
    if rec[50] & FLAG_COMMIT != 0 {
        let well_formed = rec[50] == FLAG_COMMIT
            && rec[51..].iter().all(|&b| b == 0)
            && le_u64(&rec[4..12]) == 0
            && le_u16(&rec[48..50]) == 0
            && le_u32(&rec[44..48]) == 0
            && le_u64(&rec[36..44]) == 0;
        if !well_formed {
            return Err(Error::Corrupt("malformed commit record"));
        }
        return Ok(Record::Commit {
            lamport: le_u64(&rec[12..20]),
            node_id: le_u128(&rec[20..36]),
        });
    }
    Ok(Record::Data(ChangeEvent::decode(rec)?))
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

/// Builds a 32-byte segment header.
///
/// Layout: magic(4) version(2) reserved(2) node_id(16) generation(4)
/// crc32c(4). The CRC covers the first 28 bytes.
pub(crate) fn encode_header(
    magic: [u8; 4],
    node_id: u128,
    generation: u32,
) -> [u8; HEADER_LEN as usize] {
    let mut h = [0u8; HEADER_LEN as usize];
    h[0..4].copy_from_slice(&magic);
    h[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    h[8..24].copy_from_slice(&node_id.to_le_bytes());
    h[24..28].copy_from_slice(&generation.to_le_bytes());
    let crc = crc32c(&h[..28]);
    h[28..32].copy_from_slice(&crc.to_le_bytes());
    h
}

/// Validates a segment header. Returns the node id and generation it records.
pub(crate) fn check_header(
    h: &[u8; HEADER_LEN as usize],
    magic: [u8; 4],
) -> Result<(u128, u32), Error> {
    if h[..4] != magic[..] {
        return Err(Error::Corrupt("segment magic mismatch"));
    }
    if le_u32(&h[28..32]) != crc32c(&h[..28]) {
        return Err(Error::Corrupt("segment header checksum mismatch"));
    }
    if le_u16(&h[4..6]) != FORMAT_VERSION {
        return Err(Error::Corrupt("unsupported format version"));
    }
    if h[6..8].iter().any(|&b| b != 0) {
        return Err(Error::Corrupt("segment header reserved bytes"));
    }
    Ok((le_u128(&h[8..24]), le_u32(&h[24..28])))
}

/// One entry in the Sync Segment: a committed write or delete.
///
/// The value lives in the State Segment at `payload_offset`. That record
/// also stores the full key, so `key_hash` is only a fast filter.
///
/// `(lamport, node_id)` is the total order used for conflict resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeEvent {
    /// FNV-1a 64 hash of the key.
    pub key_hash: u64,
    /// Length of the key in bytes.
    pub key_len: u16,
    /// Lamport timestamp of the write.
    pub lamport: u64,
    /// Node that issued the write.
    pub node_id: u128,
    /// Offset of the state record in the State Segment.
    pub payload_offset: u64,
    /// Length of the value in bytes. Zero for tombstones.
    pub payload_len: u32,
    /// `true` if this event deletes the key.
    pub tombstone: bool,
}

impl ChangeEvent {
    /// Serializes into a fixed 56-byte record.
    ///
    /// Layout: crc32c(4) key_hash(8) lamport(8) node_id(16) payload_offset(8)
    /// payload_len(4) key_len(2) flags(1) zero padding(5).
    /// The CRC covers bytes 4..56.
    pub(crate) fn encode(&self) -> [u8; SYNC_RECORD_LEN] {
        let mut rec = [0u8; SYNC_RECORD_LEN];
        rec[4..12].copy_from_slice(&self.key_hash.to_le_bytes());
        rec[12..20].copy_from_slice(&self.lamport.to_le_bytes());
        rec[20..36].copy_from_slice(&self.node_id.to_le_bytes());
        rec[36..44].copy_from_slice(&self.payload_offset.to_le_bytes());
        rec[44..48].copy_from_slice(&self.payload_len.to_le_bytes());
        rec[48..50].copy_from_slice(&self.key_len.to_le_bytes());
        if self.tombstone {
            rec[50] = FLAG_TOMBSTONE;
        }
        let crc = crc32c(&rec[4..]);
        rec[0..4].copy_from_slice(&crc.to_le_bytes());
        rec
    }

    /// Parses and checksum-verifies a sync record.
    pub(crate) fn decode(rec: &[u8; SYNC_RECORD_LEN]) -> Result<Self, Error> {
        if le_u32(&rec[0..4]) != crc32c(&rec[4..]) {
            return Err(Error::Corrupt("sync record checksum mismatch"));
        }
        let flags = rec[50];
        if flags & !FLAG_TOMBSTONE != 0 || rec[51..].iter().any(|&b| b != 0) {
            return Err(Error::Corrupt("sync record flags or padding"));
        }
        Ok(Self {
            key_hash: le_u64(&rec[4..12]),
            lamport: le_u64(&rec[12..20]),
            node_id: le_u128(&rec[20..36]),
            payload_offset: le_u64(&rec[36..44]),
            payload_len: le_u32(&rec[44..48]),
            key_len: le_u16(&rec[48..50]),
            tombstone: flags & FLAG_TOMBSTONE != 0,
        })
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every possible 56-byte sync record is either decoded or rejected. None
    /// panics.
    #[kani::proof]
    fn decode_record_never_panics() {
        let rec: [u8; SYNC_RECORD_LEN] = kani::any();
        let _ = decode_record(&rec);
    }

    /// Every possible 32-byte header is either accepted or rejected. None
    /// panics.
    #[kani::proof]
    fn check_header_never_panics() {
        let header: [u8; HEADER_LEN as usize] = kani::any();
        let _ = check_header(&header, STATE_MAGIC);
    }

    /// A record that decodes as data has a crc that matches its bytes.
    #[kani::proof]
    fn decoded_data_records_have_valid_checksums() {
        let rec: [u8; SYNC_RECORD_LEN] = kani::any();
        if let Ok(Record::Data(_)) = decode_record(&rec) {
            assert_eq!(le_u32(&rec[0..4]), crc32c(&rec[4..]));
        }
    }
}
