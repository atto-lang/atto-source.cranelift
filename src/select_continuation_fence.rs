//! Host continuation fence, not compiler-lowered guest scheduling.
use std::sync::{Arc, Mutex, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    Closed,
    StaleGeneration,
    Capacity,
    IdentityExhausted,
}

#[derive(Default)]
struct Slot {
    id: u64,
    generation: u64,
    active: bool,
}

struct State {
    generation: u64,
    next_id: u64,
    slots: Vec<Slot>,
    free: Vec<usize>,
    executing: usize,
    closed: bool,
}

impl State {
    fn clear(&mut self, index: usize) {
        debug_assert!(self.slots[index].active);
        self.slots[index] = Slot::default();
        self.free.push(index);
    }

    fn clear_pending(&mut self) {
        for i in 0..self.slots.len() {
            if self.slots[i].active {
                self.clear(i);
            }
        }
    }

    fn matches(&self, ticket: &Ticket) -> bool {
        self.slots.get(ticket.index).is_some_and(|slot| {
            slot.active && slot.id == ticket.id && slot.generation == ticket.generation
        })
    }
}

pub struct Fence {
    state: Arc<Mutex<State>>,
}

#[derive(Clone)]
pub struct Ticket {
    owner: Weak<Mutex<State>>,
    index: usize,
    id: u64,
    generation: u64,
}

impl Ticket {
    pub fn close(&self) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        let mut state = owner.lock().expect("continuation fence poisoned");
        if !state.matches(self) {
            return false;
        }
        state.clear(self.index);
        true
    }
}

/// Non-cloneable execution credit. Drop releases only after callback return.
/// This type does not itself schedule work or guarantee actor-lane affinity.
pub struct Execution {
    owner: Arc<Mutex<State>>,
    generation: u64,
}

impl Execution {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for Execution {
    fn drop(&mut self) {
        let mut state = self.owner.lock().expect("continuation fence poisoned");
        state.executing = state
            .executing
            .checked_sub(1)
            .expect("execution credit underflow");
    }
}

impl Drop for Fence {
    fn drop(&mut self) {
        // Pending tickets must not outlive their fence merely because an
        // executing lease retains the allocation for physical retirement.
        let _ = self.close();
    }
}

impl Fence {
    pub fn new(generation: u64, capacity: usize) -> Result<Self, AdmissionError> {
        if generation == 0 {
            return Err(AdmissionError::StaleGeneration);
        }
        if !(1..=4096).contains(&capacity) {
            return Err(AdmissionError::Capacity);
        }
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                generation,
                next_id: 1,
                slots: (0..capacity).map(|_| Slot::default()).collect(),
                free: (0..capacity).rev().collect(),
                executing: 0,
                closed: false,
            })),
        })
    }

    pub fn register(&self, generation: u64) -> Result<Ticket, AdmissionError> {
        let mut state = self.state.lock().expect("continuation fence poisoned");
        if state.closed {
            return Err(AdmissionError::Closed);
        }
        if state.generation != generation {
            return Err(AdmissionError::StaleGeneration);
        }
        if state.slots.len() - state.free.len() + state.executing >= state.slots.len() {
            return Err(AdmissionError::Capacity);
        }
        let id = state.next_id;
        let next = id.checked_add(1).ok_or(AdmissionError::IdentityExhausted)?;
        let index = state.free.pop().ok_or(AdmissionError::Capacity)?;
        state.next_id = next;
        state.slots[index] = Slot {
            id,
            generation,
            active: true,
        };
        Ok(Ticket {
            owner: Arc::downgrade(&self.state),
            index,
            id,
            generation,
        })
    }

    pub fn claim(&self, ticket: &Ticket) -> Option<Execution> {
        let owner = ticket.owner.upgrade()?;
        if !Arc::ptr_eq(&owner, &self.state) {
            return None;
        }
        let mut state = self.state.lock().expect("continuation fence poisoned");
        if state.closed || state.generation != ticket.generation || !state.matches(ticket) {
            return None;
        }
        state.clear(ticket.index);
        state.executing += 1;
        Some(Execution {
            owner: Arc::clone(&self.state),
            generation: ticket.generation,
        })
    }

    pub fn advance(&self, expected: u64) -> Result<bool, AdmissionError> {
        let mut state = self.state.lock().expect("continuation fence poisoned");
        if state.closed || state.generation != expected || state.executing != 0 {
            return Ok(false);
        }
        state.generation = expected
            .checked_add(1)
            .ok_or(AdmissionError::IdentityExhausted)?;
        state.clear_pending();
        Ok(true)
    }

    pub fn close(&self) -> bool {
        let mut state = self.state.lock().expect("continuation fence poisoned");
        if state.closed {
            return false;
        }
        state.closed = true;
        state.clear_pending();
        true
    }

    pub fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().expect("continuation fence poisoned");
        (state.slots.len() - state.free.len(), state.executing)
    }

    pub fn retirement_ready(&self) -> bool {
        let state = self.state.lock().expect("continuation fence poisoned");
        state.closed && state.executing == 0 && state.free.len() == state.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_blocks_rollover_and_close_waits_for_physical_credit() {
        let fence = Fence::new(1, 2).unwrap();
        let a = fence.register(1).unwrap();
        let b = fence.register(1).unwrap();
        let execution = fence.claim(&a).unwrap();
        assert_eq!(execution.generation(), 1);
        assert_eq!(fence.counts(), (1, 1));
        assert!(matches!(fence.register(1), Err(AdmissionError::Capacity)));
        assert_eq!(fence.advance(1), Ok(false));
        assert!(fence.claim(&a).is_none());
        assert!(!a.close());
        assert!(fence.close());
        assert!(!b.close());
        assert!(!fence.retirement_ready());
        drop(execution);
        assert_eq!(fence.counts(), (0, 0));
        assert!(fence.retirement_ready());
    }

    #[test]
    fn competing_claim_and_cancel_and_reuse() {
        for _ in 0..256 {
            let fence = Fence::new(1, 1).unwrap();
            let ticket = fence.register(1).unwrap();
            let (claimed, cancelled) = std::thread::scope(|scope| {
                let a = scope.spawn(|| fence.claim(&ticket).is_some());
                let b = scope.spawn(|| ticket.close());
                (a.join().unwrap(), b.join().unwrap())
            });
            assert_ne!(claimed, cancelled);
            assert_eq!(fence.counts(), (0, 0));
            assert_eq!(fence.advance(1), Ok(true));
            let fresh = fence.register(2).unwrap();
            assert!(fence.claim(&ticket).is_none());
            assert!(!ticket.close());
            assert!(fresh.close());
        }
    }

    #[test]
    fn foreign_and_overflow_fail_without_mutation() {
        let a = Fence::new(u64::MAX, 1).unwrap();
        let b = Fence::new(u64::MAX, 1).unwrap();
        let ticket = a.register(u64::MAX).unwrap();
        assert!(b.claim(&ticket).is_none());
        assert_eq!(a.advance(u64::MAX), Err(AdmissionError::IdentityExhausted));
        assert_eq!(a.counts(), (1, 0));
        assert!(ticket.close());
    }

    #[test]
    fn dropping_owner_revokes_pending_even_with_live_execution() {
        let fence = Fence::new(1, 2).unwrap();
        let a = fence.register(1).unwrap();
        let b = fence.register(1).unwrap();
        let execution = fence.claim(&a).unwrap();
        drop(fence);
        assert!(!b.close());
        assert_eq!(execution.generation(), 1);
        drop(execution);
        assert!(!a.close());
    }

    #[test]
    fn callback_unwind_releases_lease() {
        let fence = Fence::new(1, 1).unwrap();
        let ticket = fence.register(1).unwrap();
        let result = std::panic::catch_unwind(|| {
            let _execution = fence.claim(&ticket).unwrap();
            panic!("callback failed");
        });
        assert!(result.is_err());
        assert_eq!(fence.counts(), (0, 0));
        assert_eq!(fence.advance(1), Ok(true));
    }
    #[test]
    fn million_generation_reuse_and_mixed_capacity() {
        let fence = Fence::new(1, 4).unwrap();
        let mut stale: Option<Ticket> = None;
        for generation in 1..=1_000_000 {
            let ticket = fence.register(generation).unwrap();
            let execution = fence.claim(&ticket).unwrap();
            let pending: Vec<_> = (0..3)
                .map(|_| fence.register(generation).unwrap())
                .collect();
            assert_eq!(fence.counts(), (3, 1));
            assert!(matches!(
                fence.register(generation),
                Err(AdmissionError::Capacity)
            ));
            assert_eq!(fence.advance(generation), Ok(false));
            if let Some(previous) = stale.as_ref() {
                assert!(!previous.close());
                assert!(fence.claim(previous).is_none());
            }
            for item in pending {
                assert!(item.close());
                assert!(!item.close());
            }
            drop(execution);
            assert_eq!(fence.counts(), (0, 0));
            assert_eq!(fence.advance(generation), Ok(true));
            stale = Some(ticket);
        }
        assert!(fence.close());
        assert!(fence.retirement_ready());
    }

    #[test]
    fn concurrent_owner_settlement_keeps_physical_credit() {
        use std::sync::mpsc::sync_channel;
        use std::time::Duration;

        const OWNERS: usize = 64;
        const LIMIT: Duration = Duration::from_secs(10);
        for _ in 0..10 {
            std::thread::scope(|scope| {
                let mut owners = Vec::with_capacity(OWNERS);
                let mut pending = Vec::with_capacity(OWNERS);
                let mut releases = Vec::with_capacity(OWNERS);
                let mut workers = Vec::with_capacity(OWNERS);
                let (entered_tx, entered_rx) = sync_channel(OWNERS);
                for index in 0..OWNERS {
                    let fence = Fence::new(1, 2).unwrap();
                    let ticket = fence.register(1).unwrap();
                    let execution = fence.claim(&ticket).unwrap();
                    pending.push(fence.register(1).unwrap());
                    owners.push(Some(fence));
                    let (release_tx, release_rx) = sync_channel(1);
                    releases.push(release_tx);
                    let entered = entered_tx.clone();
                    workers.push(scope.spawn(move || {
                        let result = std::panic::catch_unwind(move || {
                            let _lease = execution;
                            entered.send(index).unwrap();
                            release_rx.recv_timeout(LIMIT).unwrap();
                            if index % 2 == 0 {
                                panic!("expected callback failure");
                            }
                        });
                        assert_eq!(result.is_err(), index % 2 == 0);
                    }));
                }
                let mut seen = [false; OWNERS];
                for _ in 0..OWNERS {
                    let index = entered_rx.recv_timeout(LIMIT).unwrap();
                    assert!(!seen[index]);
                    seen[index] = true;
                }
                for index in 0..OWNERS {
                    let fence = owners[index].as_ref().unwrap();
                    assert_eq!(fence.counts(), (1, 1));
                    assert!(matches!(fence.register(1), Err(AdmissionError::Capacity)));
                    assert_eq!(fence.advance(1), Ok(false));
                    assert!(fence.claim(&pending[(index + 1) % OWNERS]).is_none());
                    if index % 2 == 0 {
                        drop(owners[index].take());
                    } else {
                        assert!(fence.close());
                        assert!(!fence.retirement_ready());
                    }
                    assert!(!pending[index].close());
                }
                for (index, (release, worker)) in
                    releases.into_iter().zip(workers).enumerate().rev()
                {
                    release.send(()).unwrap();
                    worker.join().unwrap();
                    if let Some(fence) = &owners[index] {
                        assert!(fence.retirement_ready());
                        assert_eq!(fence.counts(), (0, 0));
                    }
                    if index > 0 {
                        if let Some(peer) = &owners[index - 1] {
                            assert_eq!(peer.counts(), (0, 1));
                            assert!(!peer.retirement_ready());
                        }
                    }
                    let fresh = Fence::new(2, 2).unwrap();
                    assert!(fresh.claim(&pending[index]).is_none());
                    let ticket = fresh.register(2).unwrap();
                    let lease = fresh.claim(&ticket).unwrap();
                    drop(lease);
                    assert!(fresh.close());
                    assert!(fresh.retirement_ready());
                }
            });
        }
    }

}
