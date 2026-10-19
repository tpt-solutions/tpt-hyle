//! Wire format for deltas. Specified in `docs/wire.md`.
//!
//! All integers are little-endian. Decoding never panics. Every length is
//! checked against the bytes that actually arrived before it is used.

use crate::RiftError;

/// Magic bytes at the start of every delta.
pub const MAGIC: [u8; 4] = *b"RIFT";
/// Version of this encoding.
pub const WIRE_VERSION: u8 = 1;

/// Fixed part of one change: lamport(8) node_id(16) flags(1) key_len(2)
/// value_len(4).
const CHANGE_HEADER_LEN: usize = 31;
/// Fixed part of a delta: magic(4) version(1) generation(4) count(4).
const DELTA_HEADER_LEN: usize = 13;
/// Upper bound on changes per delta, a guard against hostile counts.
pub const MAX_CHANGES: u32 = 1 << 20;

/// One write, as it travels between replicas.
///
/// Carries the key and value, not just an offset. An offset is meaningful
/// only inside the log that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Key bytes. Never empty.
    pub key: Vec<u8>,
    /// Value bytes. Empty for tombstones.
    pub value: Vec<u8>,
    /// Lamport timestamp assigned by the writing node.
    pub lamport: u64,
    /// Node that issued the write.
    pub node_id: u128,
    /// `true` if this change deletes the key.
    pub tombstone: bool,
}

/// A batch of changes, with the generation of the log they came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    /// Generation of the sender's log at export time.
    pub generation: u32,
    /// Changes, in the sender's log order.
    pub changes: Vec<Change>,
}

impl Delta {
    /// Serializes the delta.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            DELTA_HEADER_LEN
                + self
                    .changes
                    .iter()
                    .map(|c| CHANGE_HEADER_LEN + c.key.len() + c.value.len())
                    .sum::<usize>(),
        );
        out.extend_from_slice(&MAGIC);
        out.push(WIRE_VERSION);
        out.extend_from_slice(&self.generation.to_le_bytes());
        out.extend_from_slice(&(self.changes.len() as u32).to_le_bytes());
        for c in &self.changes {
            out.extend_from_slice(&c.lamport.to_le_bytes());
            out.extend_from_slice(&c.node_id.to_le_bytes());
            out.push(u8::from(c.tombstone));
            out.extend_from_slice(&(c.key.len() as u16).to_le_bytes());
            out.extend_from_slice(&(c.value.len() as u32).to_le_bytes());
            out.extend_from_slice(&c.key);
            out.extend_from_slice(&c.value);
        }
        out
    }

    /// Parses a delta. Rejects bad magic, unknown versions, truncated or
    /// trailing bytes, and changes that break the rules.
    pub fn decode(bytes: &[u8]) -> Result<Self, RiftError> {
        let mut r = Reader { bytes, pos: 0 };
        if r.take(4)? != MAGIC {
            return Err(RiftError::Protocol("bad delta magic"));
        }
        if r.u8()? != WIRE_VERSION {
            return Err(RiftError::Protocol("unsupported wire version"));
        }
        let generation = r.u32()?;
        let count = r.u32()?;
        if count > MAX_CHANGES {
            return Err(RiftError::Protocol("too many changes in one delta"));
        }
        // Each change takes at least its header, so a count that cannot fit
        // in the remaining bytes is hostile. Reject it before allocating.
        if (count as usize).saturating_mul(CHANGE_HEADER_LEN) > bytes.len() - r.pos {
            return Err(RiftError::Protocol("change count exceeds message length"));
        }

        let mut changes = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let lamport = r.u64()?;
            let node_id = r.u128()?;
            let flags = r.u8()?;
            if flags > 1 {
                return Err(RiftError::Protocol("unknown change flags"));
            }
            let key_len = r.u16()? as usize;
            let value_len = r.u32()? as usize;
            if key_len == 0 {
                return Err(RiftError::Protocol("empty key"));
            }
            let tombstone = flags == 1;
            if tombstone && value_len != 0 {
                return Err(RiftError::Protocol("tombstone carries a value"));
            }
            let key = r.take(key_len)?.to_vec();
            let value = r.take(value_len)?.to_vec();
            changes.push(Change {
                key,
                value,
                lamport,
                node_id,
                tombstone,
            });
        }
        if r.pos != bytes.len() {
            return Err(RiftError::Protocol("trailing bytes after delta"));
        }
        Ok(Delta {
            generation,
            changes,
        })
    }
}

/// Bounds-checked cursor over received bytes.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], RiftError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(RiftError::Protocol("truncated delta"))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, RiftError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, RiftError> {
        let mut a = [0u8; 2];
        a.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(a))
    }

    fn u32(&mut self) -> Result<u32, RiftError> {
        let mut a = [0u8; 4];
        a.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(a))
    }

    fn u64(&mut self) -> Result<u64, RiftError> {
        let mut a = [0u8; 8];
        a.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(a))
    }

    fn u128(&mut self) -> Result<u128, RiftError> {
        let mut a = [0u8; 16];
        a.copy_from_slice(self.take(16)?);
        Ok(u128::from_le_bytes(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Delta {
        Delta {
            generation: 3,
            changes: vec![
                Change {
                    key: b"user:1".to_vec(),
                    value: b"alice".to_vec(),
                    lamport: 10,
                    node_id: 1,
                    tombstone: false,
                },
                Change {
                    key: b"user:2".to_vec(),
                    value: vec![],
                    lamport: 11,
                    node_id: 2,
                    tombstone: true,
                },
            ],
        }
    }

    #[test]
    fn round_trips() {
        let d = sample();
        assert_eq!(Delta::decode(&d.encode()).unwrap(), d);
        let empty = Delta::default();
        assert_eq!(Delta::decode(&empty.encode()).unwrap(), empty);
    }

    #[test]
    fn every_truncation_is_an_error_not_a_panic() {
        let bytes = sample().encode();
        for cut in 0..bytes.len() {
            assert!(Delta::decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn rejects_trailing_bytes_bad_magic_and_bad_version() {
        let mut bytes = sample().encode();
        bytes.push(0);
        assert!(Delta::decode(&bytes).is_err());

        let mut bytes = sample().encode();
        bytes[0] ^= 1;
        assert!(Delta::decode(&bytes).is_err());

        let mut bytes = sample().encode();
        bytes[4] = 99;
        assert!(Delta::decode(&bytes).is_err());
    }

    #[test]
    fn rejects_hostile_counts_without_allocating() {
        let mut bytes = sample().encode();
        bytes[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Delta::decode(&bytes).is_err());
    }

    #[test]
    fn every_single_byte_flip_decodes_or_errors_without_panicking() {
        let bytes = sample().encode();
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0xFF;
            let _ = Delta::decode(&b);
        }
    }
}
