//! FNV-1a 64, the lookup hash for keys.
//!
//! Frames are authenticated with AEAD, so no checksum is needed for integrity.
//! This hash only picks the candidates for a key lookup, and full keys are
//! always compared.

pub(crate) const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Feeds `data` into an FNV-1a 64 state.
pub(crate) fn fnv1a_update(mut hash: u64, data: &[u8]) -> u64 {
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// One-shot FNV-1a 64 of a key.
pub(crate) fn fnv1a(data: &[u8]) -> u64 {
    fnv1a_update(FNV_OFFSET, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_reference_vectors() {
        assert_eq!(fnv1a(b""), FNV_OFFSET);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
