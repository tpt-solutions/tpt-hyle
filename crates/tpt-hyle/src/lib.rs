#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
//! Sync-native, encrypted, append-only key-value store.
//!
//! A store is one file. Every write is a sealed frame: encrypted and
//! authenticated with XChaCha20-Poly1305. Writes are grouped into batches, and
//! each batch is durable once its commit frame is on disk, after one flush. The
//! log of writes is the replication feed, so no separate change tracking exists.
//!
//! The format is in `docs/format.md`. The design decisions are in `docs/adr/`.
//!
//! # Features
//!
//! - `std` (default): file storage, passphrase derivation, and the random
//!   per-open session that nonces need.
//! - `alloc`: the store itself, and in-memory storage ([`VecStorage`]). The
//!   store needs a heap for encryption and batches.
//! - neither: the storage trait and the error type only. The no_alloc store of
//!   earlier versions is gone, see ADR 0013.
//!
//! # Example
//!
//! ```
//! use tpt_hyle::{Hyle, Key, VecStorage};
//!
//! let mut db = Hyle::open_with_session(VecStorage::new(), 1, Key::from_bytes([7; 32]), [1; 16])?;
//! db.put(b"user:101", b"alice")?;
//!
//! let mut buf = [0u8; 16];
//! let n = db.get(b"user:101", &mut buf)?.unwrap();
//! assert_eq!(&buf[..n], b"alice");
//! # Ok::<(), tpt_hyle::Error>(())
//! ```

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
#[cfg(feature = "alloc")]
mod checksum;
mod error;
#[cfg(feature = "alloc")]
#[cfg(feature = "alloc")]
mod format;
#[cfg(feature = "json")]
mod json;
mod storage;
#[cfg(feature = "alloc")]
mod store;

pub use error::Error;
#[cfg(feature = "alloc")]
pub use format::{event_message, verify_event};
#[cfg(feature = "alloc")]
#[cfg(feature = "alloc")]
pub use format::{ChangeEvent, EVENT_SIG_LEN, FORMAT_VERSION, HEADER_LEN, MAX_KEY_LEN};
pub use storage::{FixedStorage, Storage};

#[cfg(feature = "alloc")]
pub use store::{Batch, Events, Hyle, Key};

#[cfg(feature = "std")]
pub use store::file_node_id;

#[cfg(feature = "alloc")]
pub use storage::VecStorage;

#[cfg(feature = "std")]
pub use storage::FileStorage;
