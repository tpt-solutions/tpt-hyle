//! The byte-region abstraction the engine is written against.
//!
//! The engine never touches files directly. It only sees a [`Storage`],
//! which keeps the core free of I/O and lets tests inject faults.

use crate::Error;

/// An append-only byte region with explicit truncation.
///
/// Reads take `&mut self` because seeking file-backed storage mutates its
/// cursor. Every append is followed by `flush` before the engine relies on
/// it being durable.
pub trait Storage {
    /// Current length in bytes.
    fn len(&self) -> u64;

    /// `true` when the region holds no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fills `buf` from `offset`. Fails if the range is past `len()`.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error>;

    /// Appends `data` and returns the offset where it starts.
    fn append(&mut self, data: &[u8]) -> Result<u64, Error>;

    /// Shrinks the region to `len` bytes. Growing is not allowed.
    fn truncate(&mut self, len: u64) -> Result<(), Error>;

    /// Makes all prior appends and truncations durable.
    fn flush(&mut self) -> Result<(), Error>;
}

/// Bounds check shared by all backends.
pub(crate) fn check_range(len: u64, offset: u64, count: u64) -> Result<(), Error> {
    match offset.checked_add(count) {
        Some(end) if end <= len => Ok(()),
        _ => Err(Error::OutOfBounds {
            offset,
            requested: count,
            len,
        }),
    }
}

fn check_truncate(current: u64, target: u64) -> Result<(), Error> {
    if target > current {
        return Err(Error::OutOfBounds {
            offset: target,
            requested: 0,
            len: current,
        });
    }
    Ok(())
}

/// Fixed-capacity in-memory backend with no heap allocation.
///
/// This is the backend for the `no_alloc` path. `N` is the segment capacity.
#[derive(Debug, Clone)]
pub struct FixedStorage<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedStorage<N> {
    /// An empty region.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    /// Raw access to the written bytes. Intended for tests and fault
    /// injection, not for normal use.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf[..self.len]
    }
}

impl<const N: usize> Default for FixedStorage<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Storage for FixedStorage<N> {
    fn len(&self) -> u64 {
        self.len as u64
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        check_range(self.len(), offset, buf.len() as u64)?;
        let start = offset as usize;
        buf.copy_from_slice(&self.buf[start..start + buf.len()]);
        Ok(())
    }

    fn append(&mut self, data: &[u8]) -> Result<u64, Error> {
        let start = self.len;
        let end = start
            .checked_add(data.len())
            .filter(|&end| end <= N)
            .ok_or(Error::Full)?;
        self.buf[start..end].copy_from_slice(data);
        self.len = end;
        Ok(start as u64)
    }

    fn truncate(&mut self, len: u64) -> Result<(), Error> {
        check_truncate(self.len(), len)?;
        self.len = len as usize;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

/// Growable in-memory backend. Available with the `alloc` feature.
#[cfg(feature = "alloc")]
#[derive(Debug, Default, Clone)]
pub struct VecStorage {
    buf: alloc::vec::Vec<u8>,
}

#[cfg(feature = "alloc")]
impl VecStorage {
    /// An empty region.
    pub const fn new() -> Self {
        Self {
            buf: alloc::vec::Vec::new(),
        }
    }

    /// Raw access to the written bytes. Intended for tests and fault
    /// injection, not for normal use.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

#[cfg(feature = "alloc")]
impl Storage for VecStorage {
    fn len(&self) -> u64 {
        self.buf.len() as u64
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        check_range(self.len(), offset, buf.len() as u64)?;
        let start = offset as usize;
        buf.copy_from_slice(&self.buf[start..start + buf.len()]);
        Ok(())
    }

    fn append(&mut self, data: &[u8]) -> Result<u64, Error> {
        let start = self.len();
        self.buf.extend_from_slice(data);
        Ok(start)
    }

    fn truncate(&mut self, len: u64) -> Result<(), Error> {
        check_truncate(self.len(), len)?;
        self.buf.truncate(len as usize);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

/// File-backed storage. Available with the `std` feature.
///
/// `flush` calls `sync_data`, so a successful flush means the bytes have
/// reached the device.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct FileStorage {
    file: std::fs::File,
    len: u64,
    // Appends not yet written to the file. They sit at the end of the
    // logical length, and `spill` writes them out before any read, truncate
    // or flush.
    buffered: Vec<u8>,
}

#[cfg(feature = "std")]
impl FileStorage {
    /// Opens or creates the file at `path`, keeping any existing contents.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            file,
            len,
            buffered: Vec::new(),
        })
    }

    /// Creates the file at `path`, discarding any existing contents.
    pub fn create(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            file,
            len: 0,
            buffered: Vec::new(),
        })
    }
}

#[cfg(feature = "std")]
impl FileStorage {
    /// Writes buffered appends to the file at their offsets.
    fn spill(&mut self) -> Result<(), Error> {
        use std::io::{Seek, SeekFrom, Write};
        if self.buffered.is_empty() {
            return Ok(());
        }
        let start = self.len - self.buffered.len() as u64;
        self.file.seek(SeekFrom::Start(start))?;
        self.file.write_all(&self.buffered)?;
        self.buffered.clear();
        Ok(())
    }
}

#[cfg(feature = "std")]
impl Storage for FileStorage {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        use std::io::{Read, Seek, SeekFrom};
        check_range(self.len, offset, buf.len() as u64)?;
        self.spill()?;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(())
    }

    fn append(&mut self, data: &[u8]) -> Result<u64, Error> {
        let start = self.len;
        self.buffered.extend_from_slice(data);
        self.len += data.len() as u64;
        Ok(start)
    }

    fn truncate(&mut self, len: u64) -> Result<(), Error> {
        check_truncate(self.len, len)?;
        self.spill()?;
        self.file.set_len(len)?;
        self.len = len;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.spill()?;
        self.file.sync_data()?;
        Ok(())
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Any read on a fixed storage either fills the buffer from inside the
    /// written region, or returns an error. It never reads past the end.
    #[kani::proof]
    fn fixed_read_stays_in_bounds() {
        let mut s = FixedStorage::<16>::new();
        let written: usize = kani::any();
        kani::assume(written <= 16);
        let _ = s.append(&[7u8; 16][..written]);

        let offset: u64 = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 8);
        let mut buf = [0u8; 8];
        let result = s.read_at(offset, &mut buf[..len]);
        if result.is_ok() {
            assert!(offset as u128 + len as u128 <= s.len() as u128);
        }
    }

    /// Any truncation either shrinks the region or is refused, and a refused
    /// truncation leaves the length unchanged.
    #[kani::proof]
    fn truncate_never_grows() {
        let mut s = FixedStorage::<16>::new();
        let _ = s.append(&[1u8; 8]);
        let before = s.len();
        let target: u64 = kani::any();
        let result = s.truncate(target);
        if result.is_ok() {
            assert!(s.len() <= before);
        } else {
            assert_eq!(s.len(), before);
        }
    }
}
