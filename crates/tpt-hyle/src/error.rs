//! Error type shared by the whole core.

use core::fmt;

/// Everything that can go wrong in the core store.
///
/// Corruption is always reported as [`Error::Corrupt`]; the engine never
/// panics on bad on-disk bytes.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The backend has no room left for the append.
    Full,
    /// A read or truncate reached past the end of a segment.
    OutOfBounds {
        /// Requested start offset.
        offset: u64,
        /// Requested byte count.
        requested: u64,
        /// Current segment length.
        len: u64,
    },
    /// On-disk data failed validation. The string names the failed check.
    Corrupt(&'static str),
    /// The segment was written by a store with a different node id.
    NodeIdMismatch {
        /// Node id recorded in the segment header.
        stored: u128,
        /// Node id passed to `open`.
        expected: u128,
    },
    /// Keys must be non-empty.
    EmptyKey,
    /// Keys are limited to [`crate::MAX_KEY_LEN`] bytes.
    KeyTooLong,
    /// Values are limited to `u32::MAX` bytes.
    ValueTooLarge,
    /// The caller's output buffer cannot hold the value.
    BufferTooSmall {
        /// Bytes needed to hold the value.
        needed: usize,
    },
    /// A sync cursor is past the end or not on a record boundary.
    BadCursor(u64),
    /// The Lamport clock would overflow `u64`.
    LamportOverflow,
    /// An earlier write failed part-way through. Reopen the store so that
    /// recovery can repair the tail before writing again.
    Poisoned,
    /// Compaction needs empty target segments.
    TargetNotEmpty,
    /// A tombstone carries no value. A remote write violated that.
    TombstoneWithValue,
    /// JSON encoding or decoding failed.
    #[cfg(feature = "json")]
    Json(alloc::string::String),
    /// An I/O error from the file backend.
    #[cfg(feature = "std")]
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Full => write!(f, "storage is full"),
            Error::OutOfBounds {
                offset,
                requested,
                len,
            } => write!(
                f,
                "access of {requested} bytes at offset {offset} is past segment length {len}"
            ),
            Error::Corrupt(what) => write!(f, "corrupt store: {what}"),
            Error::NodeIdMismatch { stored, expected } => {
                write!(f, "store belongs to node {stored:#x}, not {expected:#x}")
            }
            Error::EmptyKey => write!(f, "keys must not be empty"),
            Error::KeyTooLong => write!(f, "key exceeds the maximum length"),
            Error::ValueTooLarge => write!(f, "value exceeds the maximum length"),
            Error::BufferTooSmall { needed } => {
                write!(f, "output buffer too small, {needed} bytes needed")
            }
            Error::BadCursor(c) => write!(f, "sync cursor {c} is not a record boundary"),
            Error::LamportOverflow => write!(f, "lamport clock overflowed"),
            Error::Poisoned => write!(f, "a previous write failed; reopen the store"),
            Error::TargetNotEmpty => write!(f, "compaction target segments must be empty"),
            Error::TombstoneWithValue => write!(f, "a tombstone must not carry a value"),
            #[cfg(feature = "json")]
            Error::Json(msg) => write!(f, "JSON error: {msg}"),
            #[cfg(feature = "std")]
            Error::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
