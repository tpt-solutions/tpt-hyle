//! Wire format for deltas. Specified in `docs/wire.md`.
//!
//! All integers are little-endian. Decoding never panics. Every length is
//! checked against the bytes that actually arrived before it is used.

use crate::RiftError;

/// Magic bytes at the start of every delta.
pub const MAGIC: [u8; 4] = *b"RIFT";
/// Version of this encoding.
pub const WIRE_VERSION: u8 = 4;

/// Fixed part of one change: lamport(8) node_id(16) flags(1) key_len(2)
/// value_len(4). The signature follows the value.
const CHANGE_HEADER_LEN: usize = 31;
/// Length of the Ed25519 signature that ends every change.
const SIGNATURE_LEN: usize = 64;
/// Fixed part of a delta: magic(4) version(1) generation(4) count(4)
/// control(1). Control fields follow, as the control bits say.
const DELTA_HEADER_LEN: usize = 14;
/// Control bit: a `caught_up` offset follows.
const CONTROL_CAUGHT_UP: u8 = 0b01;
/// Control bit: an `ack` follows.
const CONTROL_ACK: u8 = 0b10;
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
    /// Ed25519 signature by `node_id` over the event. Peers check it with the
    /// directory. See `tpt_hyle::event_message`.
    pub signature: [u8; 64],
}

/// A client's acknowledgement that it is caught up with the hub.
///
/// It means: every change this client wrote with a lamport at or below
/// `lamport` is already on the hub, and the client has applied every hub change
/// before `offset`. The hub uses acks to decide which tombstones are safe to
/// collect. See `docs/adr/0014-tombstone-collection.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ack {
    /// The hub log offset the client has applied through. It comes from the
    /// hub's `caught_up` marker.
    pub offset: u64,
    /// The client's Lamport clock, read before its last complete export.
    pub lamport: u64,
    /// Ed25519 signature by the client's node over the generation, offset and
    /// lamport. See `tpt_hyle::verify_ack`.
    pub signature: [u8; 64],
}

/// A batch of changes, with the generation of the log they came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    /// Generation of the sender's log at export time.
    pub generation: u32,
    /// Changes, in the sender's log order.
    pub changes: Vec<Change>,
    /// Set by the hub when it has sent everything in its log up to this offset.
    pub caught_up: Option<u64>,
    /// Set by a client to acknowledge a `caught_up` marker.
    pub ack: Option<Ack>,
}

impl Delta {
    /// Serializes the delta.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            DELTA_HEADER_LEN
                + 16
                + self
                    .changes
                    .iter()
                    .map(|c| CHANGE_HEADER_LEN + c.key.len() + c.value.len() + SIGNATURE_LEN)
                    .sum::<usize>(),
        );
        out.extend_from_slice(&MAGIC);
        out.push(WIRE_VERSION);
        out.extend_from_slice(&self.generation.to_le_bytes());
        out.extend_from_slice(&(self.changes.len() as u32).to_le_bytes());
        let mut control = 0u8;
        if self.caught_up.is_some() {
            control |= CONTROL_CAUGHT_UP;
        }
        if self.ack.is_some() {
            control |= CONTROL_ACK;
        }
        out.push(control);
        if let Some(offset) = self.caught_up {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        if let Some(ack) = self.ack {
            out.extend_from_slice(&ack.offset.to_le_bytes());
            out.extend_from_slice(&ack.lamport.to_le_bytes());
            out.extend_from_slice(&ack.signature);
        }
        for c in &self.changes {
            out.extend_from_slice(&c.lamport.to_le_bytes());
            out.extend_from_slice(&c.node_id.to_le_bytes());
            out.push(u8::from(c.tombstone));
            out.extend_from_slice(&(c.key.len() as u16).to_le_bytes());
            out.extend_from_slice(&(c.value.len() as u32).to_le_bytes());
            out.extend_from_slice(&c.key);
            out.extend_from_slice(&c.value);
            out.extend_from_slice(&c.signature);
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
        let control = r.u8()?;
        if control & !(CONTROL_CAUGHT_UP | CONTROL_ACK) != 0 {
            return Err(RiftError::Protocol("unknown control bits"));
        }
        let caught_up = if control & CONTROL_CAUGHT_UP != 0 {
            Some(r.u64()?)
        } else {
            None
        };
        let ack = if control & CONTROL_ACK != 0 {
            Some(Ack {
                offset: r.u64()?,
                lamport: r.u64()?,
                signature: r.sig()?,
            })
        } else {
            None
        };
        if count > MAX_CHANGES {
            return Err(RiftError::Protocol("too many changes in one delta"));
        }
        // Each change takes at least its header, so a count that cannot fit
        // in the remaining bytes is hostile. Reject it before allocating.
        if (count as usize).saturating_mul(CHANGE_HEADER_LEN + SIGNATURE_LEN) > bytes.len() - r.pos
        {
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
            let mut signature = [0u8; SIGNATURE_LEN];
            signature.copy_from_slice(r.take(SIGNATURE_LEN)?);
            changes.push(Change {
                key,
                value,
                lamport,
                node_id,
                tombstone,
                signature,
            });
        }
        if r.pos != bytes.len() {
            return Err(RiftError::Protocol("trailing bytes after delta"));
        }
        Ok(Delta {
            generation,
            changes,
            caught_up,
            ack,
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

    fn sig(&mut self) -> Result<[u8; 64], RiftError> {
        let mut a = [0u8; 64];
        a.copy_from_slice(self.take(64)?);
        Ok(a)
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every 48-byte message is parsed or rejected. None panics. Such a
    /// message is too short to hold a change, so it covers the header and
    /// control paths.
    #[kani::proof]
    #[kani::unwind(2)]
    fn decode_never_panics_on_short_input() {
        let bytes: [u8; 48] = kani::any();
        let _ = Delta::decode(&bytes);
    }

    /// Every 128-byte message, which can hold one change, is parsed or
    /// rejected. None panics.
    #[kani::proof]
    #[kani::unwind(3)]
    fn decode_never_panics_on_one_change() {
        let bytes: [u8; 128] = kani::any();
        let _ = Delta::decode(&bytes);
    }

    /// The encoding is canonical: a message that decodes re-encodes to the
    /// exact same bytes. So a peer cannot send two encodings of one delta,
    /// and no byte is ignored.
    #[kani::proof]
    #[kani::unwind(3)]
    fn accepted_delta_reencodes_identically() {
        let bytes: [u8; 128] = kani::any();
        if let Ok(delta) = Delta::decode(&bytes) {
            assert_eq!(delta.encode(), bytes.to_vec());
        }
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
                    signature: [1; 64],
                },
                Change {
                    key: b"user:2".to_vec(),
                    value: vec![],
                    lamport: 11,
                    node_id: 2,
                    tombstone: true,
                    signature: [2; 64],
                },
            ],
            caught_up: None,
            ack: None,
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
    fn round_trips_marker_and_ack() {
        let marked = Delta {
            caught_up: Some(4096),
            ..sample()
        };
        assert_eq!(Delta::decode(&marked.encode()).unwrap(), marked);

        let acked = Delta {
            changes: vec![],
            ack: Some(Ack {
                offset: 4096,
                lamport: 77,
                signature: [9; 64],
            }),
            ..sample()
        };
        assert_eq!(Delta::decode(&acked.encode()).unwrap(), acked);

        let both = Delta {
            caught_up: Some(1),
            ack: Some(Ack {
                offset: 2,
                lamport: 3,
                signature: [8; 64],
            }),
            ..sample()
        };
        assert_eq!(Delta::decode(&both.encode()).unwrap(), both);
    }

    #[test]
    fn rejects_unknown_control_bits() {
        let mut bytes = sample().encode();
        bytes[13] |= 0b1000;
        assert!(Delta::decode(&bytes).is_err());
    }

    #[test]
    fn version_one_is_refused() {
        let mut bytes = sample().encode();
        bytes[4] = 1;
        assert!(Delta::decode(&bytes).is_err());
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
