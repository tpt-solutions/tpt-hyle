//! Client acknowledgements, kept by the hub, and the tombstone bound they allow.
//!
//! A client acknowledges a position in the hub's log once it has applied
//! everything before that position, and has pushed everything it wrote up to
//! its Lamport clock at that moment. A tombstone at or below every client's
//! acknowledged position can then be dropped by compaction, because no client
//! can still bring back an older value. See `docs/adr/0014-tombstone-collection.md`.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tpt_rift::Ack;

/// What the hub last accepted from one client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The hub generation the ack refers to.
    pub generation: u32,
    /// The hub log offset the client has applied through.
    pub offset: u64,
    /// The client's clock at that point.
    pub lamport: u64,
}

/// The hub's position, used to check an ack before it is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubPosition {
    /// The hub's current generation.
    pub generation: u32,
    /// The hub's log end.
    pub end: u64,
    /// The hub's Lamport clock.
    pub lamport: u64,
}

/// The lowest position that every configured client has acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    /// Lowest acknowledged log offset. A tombstone must end at or before it.
    pub offset: u64,
    /// Lowest acknowledged clock. A tombstone's lamport must be at or below it.
    pub lamport: u64,
}

/// Acknowledgements, one per client token, optionally saved to a file.
#[derive(Debug, Clone)]
pub struct Acks {
    path: Option<PathBuf>,
    /// Token hashes, lowercase hex, of every configured client.
    tokens: Vec<String>,
    entries: BTreeMap<String, Entry>,
}

impl Acks {
    /// Acknowledgements that live only in memory, for the given clients.
    pub fn in_memory(tokens: Vec<String>) -> Self {
        Self {
            path: None,
            tokens,
            entries: BTreeMap::new(),
        }
    }

    /// Loads acknowledgements from `path`. A missing file means none yet.
    pub fn open(path: &Path, tokens: Vec<String>) -> io::Result<Self> {
        let entries = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            path: Some(path.to_path_buf()),
            tokens,
            entries,
        })
    }

    /// Records `ack` from the client whose token hashes to `token`. Returns
    /// whether it was recorded.
    ///
    /// An ack is ignored if it names another generation, lies past the hub's
    /// log or clock, or moves backwards within the current generation. Only
    /// the stored value changes, so an ignored ack is harmless. The client
    /// sends it again with its next round.
    pub fn record(
        &mut self,
        token: &str,
        hub: HubPosition,
        generation: u32,
        ack: Ack,
    ) -> io::Result<bool> {
        if generation != hub.generation || ack.offset > hub.end || ack.lamport > hub.lamport {
            return Ok(false);
        }
        if let Some(old) = self.entries.get(token) {
            if old.generation == hub.generation
                && (ack.offset < old.offset || ack.lamport < old.lamport)
            {
                return Ok(false);
            }
        }
        self.entries.insert(
            token.to_string(),
            Entry {
                generation: hub.generation,
                offset: ack.offset,
                lamport: ack.lamport,
            },
        );
        self.save()?;
        Ok(true)
    }

    /// The token hashes of the configured clients.
    pub fn tokens(&self) -> &[String] {
        &self.tokens
    }

    /// The lowest position every configured client has acknowledged in
    /// `generation`. `None` if there are no clients, or if any client has no
    /// ack in that generation. Without every client's ack nothing is safe to
    /// collect.
    pub fn bound(&self, generation: u32) -> Option<Bound> {
        if self.tokens.is_empty() {
            return None;
        }
        let mut bound = Bound {
            offset: u64::MAX,
            lamport: u64::MAX,
        };
        for token in &self.tokens {
            let entry = self.entries.get(token)?;
            if entry.generation != generation {
                return None;
            }
            bound.offset = bound.offset.min(entry.offset);
            bound.lamport = bound.lamport.min(entry.lamport);
        }
        Some(bound)
    }

    /// Writes the file atomically: a temporary file, synced, then renamed.
    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }
}

/// The acknowledgement file that goes with a store file: `<store>.acks`.
pub fn acks_path(store: &Path) -> PathBuf {
    PathBuf::from(format!("{}.acks", store.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUB: HubPosition = HubPosition {
        generation: 2,
        end: 1000,
        lamport: 50,
    };

    fn ack(offset: u64, lamport: u64) -> Ack {
        Ack {
            offset,
            lamport,
            signature: [0; 64],
        }
    }

    #[test]
    fn an_ack_is_recorded_and_moves_only_forward() {
        let mut acks = Acks::in_memory(vec!["t".into()]);
        assert!(acks.record("t", HUB, 2, ack(500, 10)).unwrap());
        assert!(
            !acks.record("t", HUB, 2, ack(400, 10)).unwrap(),
            "offset went back"
        );
        assert!(
            !acks.record("t", HUB, 2, ack(500, 9)).unwrap(),
            "clock went back"
        );
        assert!(acks.record("t", HUB, 2, ack(900, 20)).unwrap());
        assert_eq!(acks.bound(2).unwrap().offset, 900);
    }

    #[test]
    fn an_ack_outside_the_hub_is_refused() {
        let mut acks = Acks::in_memory(vec!["t".into()]);
        assert!(
            !acks.record("t", HUB, 1, ack(10, 10)).unwrap(),
            "other generation"
        );
        assert!(
            !acks.record("t", HUB, 2, ack(1001, 10)).unwrap(),
            "past the log end"
        );
        assert!(
            !acks.record("t", HUB, 2, ack(10, 51)).unwrap(),
            "clock ahead of the hub"
        );
        assert_eq!(acks.bound(2), None);
    }

    #[test]
    fn a_new_generation_replaces_the_old_ack() {
        let mut acks = Acks::in_memory(vec!["t".into()]);
        acks.record("t", HUB, 2, ack(900, 20)).unwrap();
        let next = HubPosition {
            generation: 3,
            end: 100,
            lamport: 50,
        };
        assert!(acks.record("t", next, 3, ack(50, 5)).unwrap());
        assert_eq!(
            acks.bound(3),
            Some(Bound {
                offset: 50,
                lamport: 5
            })
        );
        assert_eq!(acks.bound(2), None, "the old generation no longer counts");
    }

    #[test]
    fn the_bound_needs_every_client() {
        let mut acks = Acks::in_memory(vec!["a".into(), "b".into()]);
        acks.record("a", HUB, 2, ack(900, 20)).unwrap();
        assert_eq!(acks.bound(2), None);
        acks.record("b", HUB, 2, ack(700, 30)).unwrap();
        assert_eq!(
            acks.bound(2),
            Some(Bound {
                offset: 700,
                lamport: 20
            })
        );
    }

    #[test]
    fn no_clients_means_no_bound() {
        assert_eq!(Acks::in_memory(vec![]).bound(2), None);
    }

    #[test]
    fn acks_survive_a_reopen() {
        let dir = std::env::temp_dir().join(format!("tpt-acks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub.acks");
        let _ = std::fs::remove_file(&path);
        {
            let mut acks = Acks::open(&path, vec!["t".into()]).unwrap();
            acks.record("t", HUB, 2, ack(500, 10)).unwrap();
        }
        let reopened = Acks::open(&path, vec!["t".into()]).unwrap();
        assert_eq!(
            reopened.bound(2),
            Some(Bound {
                offset: 500,
                lamport: 10
            })
        );
        assert!(
            !dir.join("hub.acks.tmp").exists(),
            "the temp file is renamed away"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
