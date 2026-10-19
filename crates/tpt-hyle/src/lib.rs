#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
//! Sync-native, append-only micro key-value store (State + Sync segments).
//!
//! Every write appends a value to the **State Segment** and a fixed-size
//! [`ChangeEvent`] to the **Sync Segment**. The Sync Segment is the
//! replication feed, so no separate change tracking is needed.
//!
//! The on-disk format is documented in `docs/format.md`.
//!
//! # Features
//!
//! - default `std`: file-backed storage and [`Hyle::open_file`].
//! - `alloc`: growable in-memory storage ([`VecStorage`]).
//! - neither: `no_std`, no heap. Use [`FixedStorage`] for storage.
//!
//! # Example
//!
//! ```
//! use tpt_hyle::{FixedStorage, Hyle};
//!
//! let mut db = Hyle::open(FixedStorage::<4096>::new(), FixedStorage::<4096>::new(), 1)?;
//! db.put(b"user:101", b"alice")?;
//!
//! let mut buf = [0u8; 16];
//! let n = db.get(b"user:101", &mut buf)?.unwrap();
//! assert_eq!(&buf[..n], b"alice");
//! # Ok::<(), tpt_hyle::Error>(())
//! ```

#[cfg(feature = "alloc")]
extern crate alloc;

mod checksum;
mod error;
mod format;
#[cfg(feature = "json")]
mod json;
mod storage;
mod store;

pub use error::Error;
pub use format::{ChangeEvent, FORMAT_VERSION, HEADER_LEN, MAX_KEY_LEN, SYNC_RECORD_LEN};
pub use storage::{FixedStorage, Storage};
#[cfg(feature = "std")]
pub use store::file_node_id;
pub use store::{Events, Hyle};

#[cfg(feature = "alloc")]
pub use storage::VecStorage;

#[cfg(feature = "std")]
pub use storage::FileStorage;
