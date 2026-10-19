//! An in-process transport for tests and examples. Two linked ends share a
//! pair of queues. Nothing touches the network.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::transport::Transport;
use crate::wire::Delta;
use crate::RiftError;

type Queue = Arc<Mutex<VecDeque<Delta>>>;

/// One end of an in-process link. Create both ends with [`MockTransport::pair`].
#[derive(Clone)]
pub struct MockTransport {
    inbox: Queue,
    outbox: Queue,
}

impl MockTransport {
    /// Returns two linked ends. A delta sent on one arrives at the other.
    pub fn pair() -> (Self, Self) {
        let a: Queue = Arc::default();
        let b: Queue = Arc::default();
        (
            Self {
                inbox: a.clone(),
                outbox: b.clone(),
            },
            Self {
                inbox: b,
                outbox: a,
            },
        )
    }
}

fn lock(q: &Queue) -> Result<std::sync::MutexGuard<'_, VecDeque<Delta>>, RiftError> {
    q.lock()
        .map_err(|_| RiftError::Transport("mock queue poisoned".into()))
}

#[async_trait]
impl Transport for MockTransport {
    async fn send(&self, delta: Delta) -> Result<(), RiftError> {
        lock(&self.outbox)?.push_back(delta);
        Ok(())
    }

    async fn receive(&self) -> Result<Option<Delta>, RiftError> {
        Ok(lock(&self.inbox)?.pop_front())
    }
}
