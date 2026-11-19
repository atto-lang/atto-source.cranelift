//! Host-only supervisor STOP receipt ledger.
//!
//! This does not implement guest `define supervisor`, actor scheduling, or
//! CONTROL lowering. The owning runtime must independently prove physical
//! finalization and external host STOP before asserting these transitions.
//! All identity/ticket fields are private to trusted Cranelift host code.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

// Nonsecret, process-unique ledger instance discriminator. Logical child IDs,
// actor IDs, and turn counters can repeat across newly constructed registries.
// If this counter exhausts, never silently reuse a domain in this process.
static NEXT_LEDGER_DOMAIN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActorKey {
    id: u64,
    generation: u64,
}

impl ActorKey {
    pub(crate) fn new(id: u64, generation: u64) -> Result<Self, &'static str> {
        if id == 0 || generation == 0 {
            return Err("invalid actor identity or incarnation");
        }
        Ok(Self { id, generation })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChildKey {
    domain: u64,
    supervisor: u64,
    group: u64,
    logical_id: u64,
    actor: ActorKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StopTicket {
    child: ChildKey,
    turn: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Active,
    Finalized,
    Stopping,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Receipt {
    Pending,
    Applied,
    Uncertain,
    Acknowledged,
}

struct Entry {
    child: ChildKey,
    phase: Phase,
    receipt: Receipt,
    ticket: Option<StopTicket>,
    invoked: bool,
}

/// Single-writer CONTROL ledger. Exclusive `&mut self` is the mutation
/// authority; a concurrent host must place this behind its runtime gate.
pub(crate) struct SupervisorStopLedger {
    domain: u64,
    supervisor: u64,
    group: u64,
    capacity: usize,
    next_child: u64,
    next_turn: u64,
    children: BTreeMap<u64, Entry>,
}

impl SupervisorStopLedger {
    pub(crate) fn new(supervisor: u64, group: u64, capacity: usize) -> Result<Self, &'static str> {
        if supervisor == 0 || group == 0 || capacity == 0 || capacity > 65_536 {
            return Err("invalid supervisor/group or capacity");
        }
        let domain = NEXT_LEDGER_DOMAIN
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| "supervisor ledger domain exhausted")?;
        Ok(Self {
            domain,
            supervisor,
            group,
            capacity,
            next_child: 1,
            next_turn: 1,
            children: BTreeMap::new(),
        })
    }

    pub(crate) fn register(&mut self, actor: ActorKey) -> Result<ChildKey, &'static str> {
        if self.children.len() >= self.capacity {
            return Err("bounded supervisor child slots exhausted");
        }
        if self.next_child == u64::MAX {
            return Err("logical child identity exhausted");
        }
        if self.children.values().any(|e| e.child.actor == actor) {
            return Err("duplicate physical actor identity");
        }
        let child = ChildKey {
            domain: self.domain,
            supervisor: self.supervisor,
            group: self.group,
            logical_id: self.next_child,
            actor,
        };
        self.children.insert(
            child.logical_id,
            Entry {
                child,
                phase: Phase::Active,
                receipt: Receipt::Pending,
                ticket: None,
                invoked: false,
            },
        );
        self.next_child += 1;
        Ok(child)
    }

    fn entry(&self, child: ChildKey) -> Result<&Entry, &'static str> {
        if child.domain != self.domain
            || child.supervisor != self.supervisor
            || child.group != self.group
        {
            return Err("foreign supervisor or group");
        }
        match self.children.get(&child.logical_id) {
            Some(entry) if entry.child == child => Ok(entry),
            _ => Err("stale actor generation or forged logical child"),
        }
    }

    fn entry_mut(&mut self, child: ChildKey) -> Result<&mut Entry, &'static str> {
        self.entry(child)?;
        self.children
            .get_mut(&child.logical_id)
            .ok_or("missing child")
    }

    fn ticket_mut(&mut self, ticket: StopTicket) -> Result<&mut Entry, &'static str> {
        let entry = self.entry_mut(ticket.child)?;
        if entry.phase != Phase::Stopping || entry.ticket != Some(ticket) {
            return Err("stale or replayed STOP ticket");
        }
        Ok(entry)
    }

    pub(crate) fn snapshot(&self, child: ChildKey) -> Result<Phase, &'static str> {
        Ok(self.entry(child)?.phase)
    }

    pub(crate) fn assert_physically_finalized(
        &mut self,
        child: ChildKey,
    ) -> Result<(), &'static str> {
        let entry = self.entry_mut(child)?;
        if entry.phase != Phase::Active {
            return Err("duplicate or stale physical finalization");
        }
        entry.phase = Phase::Finalized;
        Ok(())
    }

    pub(crate) fn reserve_stop(&mut self, child: ChildKey) -> Result<StopTicket, &'static str> {
        if self.entry(child)?.phase != Phase::Finalized {
            return Err("STOP requires finalized physical actor");
        }
        if self.next_turn == u64::MAX {
            return Err("CONTROL turn identity exhausted");
        }
        let ticket = StopTicket {
            child,
            turn: self.next_turn,
        };
        let entry = self.entry_mut(child)?;
        entry.ticket = Some(ticket);
        entry.receipt = Receipt::Pending;
        entry.phase = Phase::Stopping;
        self.next_turn += 1;
        Ok(ticket)
    }

    pub(crate) fn claim_host_invocation(&mut self, ticket: StopTicket) -> Result<(), &'static str> {
        let entry = self.ticket_mut(ticket)?;
        if entry.invoked || entry.receipt != Receipt::Pending {
            return Err("STOP invocation already claimed or classified");
        }
        entry.invoked = true;
        Ok(())
    }

    pub(crate) fn mark_applied(&mut self, ticket: StopTicket) -> Result<(), &'static str> {
        let entry = self.ticket_mut(ticket)?;
        if !entry.invoked || entry.receipt != Receipt::Pending {
            return Err("STOP not invoked or already classified");
        }
        entry.receipt = Receipt::Applied;
        Ok(())
    }

    pub(crate) fn mark_uncertain(&mut self, ticket: StopTicket) -> Result<(), &'static str> {
        let entry = self.ticket_mut(ticket)?;
        if !entry.invoked {
            return Err("STOP not invoked");
        }
        if entry.receipt == Receipt::Uncertain {
            return Ok(());
        }
        if entry.receipt != Receipt::Pending {
            return Err("cannot downgrade a confirmed STOP");
        }
        entry.receipt = Receipt::Uncertain;
        Ok(())
    }

    pub(crate) fn receipt(&self, ticket: StopTicket) -> Result<Receipt, &'static str> {
        let entry = self.entry(ticket.child)?;
        if entry.phase != Phase::Stopping || entry.ticket != Some(ticket) {
            return Err("stale or foreign STOP receipt");
        }
        Ok(entry.receipt)
    }

    pub(crate) fn acknowledge_applied(&mut self, ticket: StopTicket) -> Result<(), &'static str> {
        let entry = self.ticket_mut(ticket)?;
        if entry.receipt != Receipt::Applied {
            return Err("unconfirmed host STOP");
        }
        entry.receipt = Receipt::Acknowledged;
        entry.phase = Phase::Stopped;
        entry.ticket = None;
        Ok(())
    }

    /// Trust boundary: caller MUST have independently verified the external
    /// STOP result for this exact incarnation; no guest may call this.
    pub(crate) fn reconcile_verified(&mut self, ticket: StopTicket) -> Result<(), &'static str> {
        let entry = self.ticket_mut(ticket)?;
        if entry.receipt != Receipt::Uncertain && entry.receipt != Receipt::Applied {
            return Err("STOP not ready for reconciliation");
        }
        entry.receipt = Receipt::Acknowledged;
        entry.phase = Phase::Stopped;
        entry.ticket = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: u64, generation: u64) -> ActorKey {
        ActorKey::new(id, generation).unwrap()
    }

    #[test]
    fn applied_stop_requires_finalization_and_exactly_one_invocation() {
        let mut l = SupervisorStopLedger::new(2, 3, 2).unwrap();
        let child = l.register(actor(4, 5)).unwrap();
        assert!(l.reserve_stop(child).is_err());
        l.assert_physically_finalized(child).unwrap();
        assert!(l.assert_physically_finalized(child).is_err());
        let t = l.reserve_stop(child).unwrap();
        assert_eq!(t.turn, 1);
        assert!(l.reserve_stop(child).is_err());
        assert!(l.mark_applied(t).is_err());
        l.claim_host_invocation(t).unwrap();
        assert!(l.claim_host_invocation(t).is_err());
        l.mark_applied(t).unwrap();
        assert!(l.mark_uncertain(t).is_err());
        l.acknowledge_applied(t).unwrap();
        assert_eq!(l.snapshot(child).unwrap(), Phase::Stopped);
        assert!(l.acknowledge_applied(t).is_err());
    }

    #[test]
    fn uncertain_stop_stays_quarantined_until_exact_host_reconciliation() {
        let mut l = SupervisorStopLedger::new(2, 3, 2).unwrap();
        let child = l.register(actor(4, 5)).unwrap();
        l.assert_physically_finalized(child).unwrap();
        let t = l.reserve_stop(child).unwrap();
        l.claim_host_invocation(t).unwrap();
        l.mark_uncertain(t).unwrap();
        l.mark_uncertain(t).unwrap();
        assert_eq!(l.receipt(t).unwrap(), Receipt::Uncertain);
        assert!(l.acknowledge_applied(t).is_err());
        assert!(l.reserve_stop(child).is_err());
        assert!(l.reconcile_verified(StopTicket { turn: 99, ..t }).is_err());
        assert!(
            l.reconcile_verified(StopTicket {
                child: ChildKey {
                    actor: actor(4, 6),
                    ..child
                },
                ..t
            })
            .is_err()
        );
        assert_eq!(l.snapshot(child).unwrap(), Phase::Stopping);
        l.reconcile_verified(t).unwrap();
        assert_eq!(l.snapshot(child).unwrap(), Phase::Stopped);
        assert!(l.reconcile_verified(t).is_err());
    }

    #[test]
    fn identical_numeric_names_in_distinct_ledgers_do_not_authorize_replay() {
        // A restarted host may legitimately reuse the same supervisor/group,
        // actor, logical child index and CONTROL turn. Instance identity must
        // still reject an old ticket before invoking any host-side effect.
        let mut old = SupervisorStopLedger::new(8, 9, 1).unwrap();
        let mut current = SupervisorStopLedger::new(8, 9, 1).unwrap();
        assert_ne!(old.domain, current.domain);
        let original = old.register(actor(2, 1)).unwrap();
        let replacement = current.register(actor(2, 1)).unwrap();
        assert_eq!(original.logical_id, replacement.logical_id);
        assert_eq!(original.actor, replacement.actor);
        old.assert_physically_finalized(original).unwrap();
        current.assert_physically_finalized(replacement).unwrap();
        let old_ticket = old.reserve_stop(original).unwrap();
        let new_ticket = current.reserve_stop(replacement).unwrap();
        assert_eq!(old_ticket.turn, new_ticket.turn);
        assert!(current.snapshot(original).is_err());
        assert!(current.claim_host_invocation(old_ticket).is_err());
        assert!(current.mark_applied(old_ticket).is_err());
        assert!(current.reconcile_verified(old_ticket).is_err());
        assert_eq!(current.receipt(new_ticket).unwrap(), Receipt::Pending);
        current.claim_host_invocation(new_ticket).unwrap();
        current.mark_applied(new_ticket).unwrap();
        current.acknowledge_applied(new_ticket).unwrap();
        assert_eq!(current.snapshot(replacement).unwrap(), Phase::Stopped);
    }

    #[test]
    fn foreign_authority_stale_actor_and_capacity_fail_closed() {
        let mut l = SupervisorStopLedger::new(8, 9, 1).unwrap();
        let key = actor(2, 1);
        let child = l.register(key).unwrap();
        assert!(l.register(key).is_err());
        assert!(l.register(actor(3, 1)).is_err());
        assert!(
            l.snapshot(ChildKey {
                supervisor: 12,
                ..child
            })
            .is_err()
        );
        assert!(l.snapshot(ChildKey { group: 12, ..child }).is_err());
        assert!(
            l.snapshot(ChildKey {
                actor: actor(2, 2),
                ..child
            })
            .is_err()
        );
        assert!(ActorKey::new(0, 1).is_err());
        assert!(SupervisorStopLedger::new(0, 1, 1).is_err());
    }
}
