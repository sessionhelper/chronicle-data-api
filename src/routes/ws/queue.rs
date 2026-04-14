//! Bounded drop-oldest queue used per WS subscriber.
//!
//! Backed by a `VecDeque` and a counter of dropped events; if the queue
//! is already at cap, the oldest element is dropped to make room.

use std::collections::VecDeque;

use crate::events::Event;

pub struct DropOldestQueue {
    cap: usize,
    inner: VecDeque<Event>,
    dropped: u64,
}

impl DropOldestQueue {
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            cap,
            inner: VecDeque::with_capacity(cap),
            dropped: 0,
        }
    }

    /// Push a new event. Returns `true` if a previous event was dropped.
    pub fn push(&mut self, e: Event) -> bool {
        let mut overflowed = false;
        if self.inner.len() >= self.cap {
            let _ = self.inner.pop_front();
            self.dropped = self.dropped.saturating_add(1);
            overflowed = true;
        }
        self.inner.push_back(e);
        overflowed
    }

    pub fn pop(&mut self) -> Option<Event> {
        self.inner.pop_front()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[cfg(test)]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ChunkUploaded, Event};
    use crate::ids::PseudoId;
    use chrono::Utc;
    use uuid::Uuid;

    fn ev() -> Event {
        Event::ChunkUploaded {
            at_ts: Utc::now(),
            data: ChunkUploaded {
                session_id: Uuid::new_v4(),
                guild_id: 1,
                pseudo_id: PseudoId::new("0123456789abcdef01234567").unwrap(),
                seq: 0,
                size_bytes: 1,
            },
        }
    }

    #[test]
    fn drops_oldest_on_overflow() {
        let mut q = DropOldestQueue::new(2);
        assert!(!q.push(ev()));
        assert!(!q.push(ev()));
        // third push overflows: oldest is dropped.
        assert!(q.push(ev()));
        assert_eq!(q.len(), 2);
        assert_eq!(q.dropped(), 1);
    }

    #[test]
    fn zero_cap_coerces_to_one() {
        let mut q = DropOldestQueue::new(0);
        q.push(ev());
        assert_eq!(q.len(), 1);
    }
}
