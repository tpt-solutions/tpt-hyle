//! Checksums used by the on-disk format. See `docs/format.md`.
//!
//! CRC-32C (Castagnoli) guards every record. FNV-1a 64 is the `key_hash`
//! stored in each sync event. Both are hand-rolled so the core stays
//! dependency-free and `no_std`.

const CRC32C_POLY_REFLECTED: u32 = 0x82F6_3B78;

const fn crc32c_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ CRC32C_POLY_REFLECTED
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

static CRC32C_TABLE: [u32; 256] = crc32c_table();

/// Raw register value to start an incremental CRC-32C.
pub(crate) const CRC_INIT: u32 = !0;

/// Feeds `data` into a raw CRC-32C register.
pub(crate) fn crc32c_update(mut reg: u32, data: &[u8]) -> u32 {
    for &byte in data {
        reg = CRC32C_TABLE[((reg ^ u32::from(byte)) & 0xFF) as usize] ^ (reg >> 8);
    }
    reg
}

/// Turns a raw register into the final CRC-32C value.
pub(crate) fn crc32c_finish(reg: u32) -> u32 {
    !reg
}

/// One-shot CRC-32C.
pub(crate) fn crc32c(data: &[u8]) -> u32 {
    crc32c_finish(crc32c_update(CRC_INIT, data))
}

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

/// One-shot FNV-1a 64, used as `key_hash`.
pub(crate) fn fnv1a(data: &[u8]) -> u64 {
    fnv1a_update(FNV_OFFSET, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_matches_standard_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0);
    }

    #[test]
    fn crc32c_incremental_equals_one_shot() {
        let reg = crc32c_update(CRC_INIT, b"12345");
        let reg = crc32c_update(reg, b"6789");
        assert_eq!(crc32c_finish(reg), crc32c(b"123456789"));
    }

    #[test]
    fn fnv1a_matches_reference_vectors() {
        assert_eq!(fnv1a(b""), FNV_OFFSET);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
