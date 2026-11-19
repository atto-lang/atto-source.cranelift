//! Scheduler-facing critical ticket substrate; **not** source-level critical
//! lowering. Oreslang Cranelift stage-0 still fails closed on critical/suspends.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, TryLockError};

const MAX_WAITERS: usize = 8192;
static NEXT_SCOPE: AtomicU64 = AtomicU64::new(0);

/// Only the gate can mint a token; a sequence from a retired scope cannot
/// authorize a replacement actor or another critical resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ticket {
    scope: u64,
    sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    Acquired,
    Queued,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request {
    pub ticket: Ticket,
    pub admission: Admission,
}

#[derive(Default)]
struct State {
    issued: u64,
    owner: Option<Ticket>,
    waiting: VecDeque<Ticket>,
    retired: bool,
}

/// No guest code runs under this short bookkeeping mutex. A queued request
/// parks its continuation in the trusted scheduler, not an OS carrier.
pub struct CriticalGate {
    scope: u64,
    state: Mutex<State>,
}

impl CriticalGate {
    pub fn new() -> Result<Self, String> {
        let old = NEXT_SCOPE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
            .map_err(|_| "critical gate scope ids exhausted".to_owned())?;
        Ok(Self {
            scope: old + 1,
            state: Mutex::new(State::default()),
        })
    }

    fn next_ticket(&self, state: &mut State) -> Result<Ticket, String> {
        state.issued = state
            .issued
            .checked_add(1)
            .ok_or_else(|| "critical ticket sequence exhausted".to_owned())?;
        Ok(Ticket {
            scope: self.scope,
            sequence: state.issued,
        })
    }

    /// Immediate nonblocking attempt. Never queues or waits on the metadata
    /// lock; caller must still validate actor/task generation before use.
    pub fn try_request(&self) -> Result<Option<Ticket>, String> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => return Err("critical gate metadata poisoned".into()),
        };
        if state.retired || state.owner.is_some() || !state.waiting.is_empty() {
            return Ok(None);
        }
        let ticket = self.next_ticket(&mut state)?;
        state.owner = Some(ticket);
        Ok(Some(ticket))
    }

    /// Admission never waits for the current guest critical body.
    pub fn request(&self) -> Result<Request, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "critical gate metadata poisoned")?;
        if state.retired {
            return Err("critical gate retired".into());
        }
        let immediate = state.owner.is_none() && state.waiting.is_empty();
        if !immediate && state.waiting.len() >= MAX_WAITERS {
            return Err("critical waiter limit exceeded".into());
        }
        let ticket = self.next_ticket(&mut state)?;
        if immediate {
            state.owner = Some(ticket);
            Ok(Request {
                ticket,
                admission: Admission::Acquired,
            })
        } else {
            state.waiting.push_back(ticket);
            Ok(Request {
                ticket,
                admission: Admission::Queued,
            })
        }
    }

    /// Release owner with FIFO handoff or cancel queued ticket. Returns the
    /// successor token to enqueue *outside* the gate. Never executes guest code.
    pub fn release(&self, ticket: Ticket) -> Result<Option<Ticket>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "critical gate metadata poisoned")?;
        if ticket.scope != self.scope || ticket.sequence == 0 || ticket.sequence > state.issued {
            return Err("foreign or invalid critical ticket".into());
        }
        if state.retired {
            return Ok(None);
        }
        if state.owner == Some(ticket) {
            state.owner = state.waiting.pop_front();
            return Ok(state.owner);
        }
        if let Some(index) = state
            .waiting
            .iter()
            .position(|candidate| *candidate == ticket)
        {
            state.waiting.remove(index);
            return Ok(None);
        }
        // A previously released/cancelled ticket is not a successful
        // release. Reject stale acknowledgements while the gate is live.
        Err("stale or already released critical ticket".into())
    }

    pub fn holds(&self, ticket: Ticket) -> Result<bool, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "critical gate metadata poisoned")?;
        Ok(!state.retired && ticket.scope == self.scope && state.owner == Some(ticket))
    }

    pub fn waiting(&self) -> Result<usize, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "critical gate metadata poisoned")?;
        Ok(state.waiting.len())
    }

    /// Caller must first quiesce any physically executing critical body.
    /// This denies stale admission but cannot interrupt an active mutation.
    pub fn retire(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "critical gate metadata poisoned")?;
        state.retired = true;
        state.owner = None;
        state.waiting.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn try_critical_does_not_wait_or_queue() {
        let gate = CriticalGate::new().unwrap();
        let first = gate.try_request().unwrap().unwrap();
        assert!(gate.holds(first).unwrap());
        assert_eq!(gate.try_request().unwrap(), None);
        assert_eq!(gate.waiting().unwrap(), 0);
        assert_eq!(gate.release(first).unwrap(), None);
        let second = gate.try_request().unwrap().unwrap();
        assert_ne!(first, second);
        assert!(gate.release(first).is_err());
        assert!(gate.holds(second).unwrap());
        let _ = gate.release(second).unwrap();
    }

    #[test]
    fn live_gate_rejects_duplicate_release_and_cancel_without_reassigning_owner() {
        let gate = CriticalGate::new().unwrap();
        let owner = gate.request().unwrap().ticket;
        let cancelled = gate.request().unwrap().ticket;
        let successor = gate.request().unwrap().ticket;
        assert_eq!(gate.release(cancelled), Ok(None));
        assert!(gate.release(cancelled).is_err(), "double cancellation acknowledged");
        assert!(gate.holds(owner).unwrap());
        assert_eq!(gate.release(owner), Ok(Some(successor)));
        assert!(gate.release(owner).is_err(), "old owner re-release acknowledged");
        assert!(gate.holds(successor).unwrap());
        assert_eq!(gate.release(successor), Ok(None));
        assert!(gate.release(successor).is_err(), "double release acknowledged");
        assert_eq!(gate.waiting(), Ok(0));
    }

    #[test]
    fn fifo_cancel_and_retire_fence() {
        let gate = CriticalGate::new().unwrap();
        let first = gate.request().unwrap();
        let cancelled = gate.request().unwrap();
        let next = gate.request().unwrap();
        assert_eq!(first.admission, Admission::Acquired);
        assert_eq!(cancelled.admission, Admission::Queued);
        assert_eq!(next.admission, Admission::Queued);
        assert_eq!(gate.release(cancelled.ticket).unwrap(), None);
        assert_eq!(gate.release(first.ticket).unwrap(), Some(next.ticket));
        assert!(!gate.holds(cancelled.ticket).unwrap());
        assert!(gate.holds(next.ticket).unwrap());
        gate.retire().unwrap();
        assert!(!gate.holds(next.ticket).unwrap());
        assert_eq!(gate.release(next.ticket).unwrap(), None);
        assert_eq!(gate.try_request().unwrap(), None);
        assert!(gate.request().is_err());
        let replacement = CriticalGate::new().unwrap();
        assert!(!replacement.holds(next.ticket).unwrap());
        assert_eq!(
            replacement.request().unwrap().admission,
            Admission::Acquired
        );
    }

    #[test]
    fn foreign_tickets_are_rejected() {
        let a = CriticalGate::new().unwrap();
        let b = CriticalGate::new().unwrap();
        let token = a.request().unwrap().ticket;
        assert!(b.release(token).is_err());
        assert!(!b.holds(token).unwrap());
        assert!(a.holds(token).unwrap());
    }

    #[test]
    fn cross_carrier_release_hands_off_without_guest_callback() {
        let gate = Arc::new(CriticalGate::new().unwrap());
        let owner = gate.request().unwrap();
        let successor = gate.request().unwrap();
        let copy = Arc::clone(&gate);
        let handed = std::thread::spawn(move || copy.release(owner.ticket).unwrap())
            .join()
            .unwrap();
        assert_eq!(handed, Some(successor.ticket));
        assert!(gate.holds(successor.ticket).unwrap());
        let _ = gate.release(successor.ticket).unwrap();
    }

    #[test]
    fn bounded_waiter_quota() {
        let gate = CriticalGate::new().unwrap();
        let first = gate.request().unwrap();
        let second = gate.request().unwrap();
        for _ in 1..MAX_WAITERS {
            assert_eq!(gate.request().unwrap().admission, Admission::Queued);
        }
        assert!(gate.request().is_err());
        assert!(gate.holds(first.ticket).unwrap());
        let _ = gate.release(second.ticket).unwrap();
        assert_eq!(gate.request().unwrap().admission, Admission::Queued);
        gate.retire().unwrap();
        assert_eq!(gate.waiting().unwrap(), 0);
    }

    #[test]
    fn cancel_racing_release_preserves_live_successor() {
        for _ in 0..128 {
            let gate = Arc::new(CriticalGate::new().unwrap());
            let owner = gate.request().unwrap();
            let cancelled = gate.request().unwrap();
            let survivor = gate.request().unwrap();
            let left = Arc::clone(&gate);
            let right = Arc::clone(&gate);
            let t1 = std::thread::spawn(move || left.release(owner.ticket).unwrap());
            let t2 = std::thread::spawn(move || right.release(cancelled.ticket).unwrap());
            t1.join().unwrap();
            t2.join().unwrap();
            assert!(!gate.holds(cancelled.ticket).unwrap());
            assert!(gate.holds(survivor.ticket).unwrap());
            let _ = gate.release(survivor.ticket).unwrap();
        }
    }
}
