//! State-region provenance v1: *receipt values*, never guest-visible authority.
//!
//! Compiler requirements name symbolic regions and current logical execution;
//! only the trusted runtime may bind concrete region and scheduler generations.
//! Receipt types remain POD values. A separate host ledger tracks live admission.
//! NO compiler lowering or state-container execution is implied by this module.
#![allow(dead_code)]

// 1.0 JSON/WASM interchange stores IDs in IEEE-754 exact integer range.
const MAX_JSON_SAFE_ID: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanKind {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanAction {
    Borrow,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateRegionLoanRequirement<'a> {
    region_binding: &'a str,
    generation_fence: bool,
    loan_id: u64,
    place: &'a str,
    kind: LoanKind,
    action: LoanAction,
}

impl<'a> StateRegionLoanRequirement<'a> {
    pub fn new(
        region_binding: &'a str,
        generation_fence: bool,
        loan_id: u64,
        place: &'a str,
        kind: LoanKind,
        action: LoanAction,
    ) -> Result<Self, &'static str> {
        if region_binding.trim().is_empty() {
            return Err("region binding required");
        }
        if !generation_fence {
            return Err("state loan must be generation fenced");
        }
        if loan_id == 0 || loan_id > MAX_JSON_SAFE_ID {
            return Err("loan id must be an exact positive JSON/WASM integer");
        }
        if place.trim().is_empty() {
            return Err("state loan place required");
        }
        Ok(Self {
            region_binding,
            generation_fence,
            loan_id,
            place,
            kind,
            action,
        })
    }

    pub fn authorizes_release(&self, release: &Self) -> bool {
        self.valid()
            && release.valid()
            && self.action == LoanAction::Borrow
            && release.action == LoanAction::Release
            && self.region_binding == release.region_binding
            && self.generation_fence == release.generation_fence
            && self.loan_id == release.loan_id
            && self.place == release.place
            && self.kind == release.kind
    }

    fn valid(&self) -> bool {
        !self.region_binding.trim().is_empty()
            && self.generation_fence
            && (1..=MAX_JSON_SAFE_ID).contains(&self.loan_id)
            && !self.place.trim().is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateRegionLoanProvenance<'a> {
    region_id: u64,
    region_generation: u64,
    runtime_domain: u64,
    execution_id: u64,
    execution_slot: u64,
    execution_generation: u64,
    loan_id: u64,
    place: &'a str,
    kind: LoanKind,
    action: LoanAction,
}

impl<'a> StateRegionLoanProvenance<'a> {
    // Private module prevents guest code from minting runtime scheduler IDs.
    // Runtime integration must pass AUTHENTICATED identity, never guest input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        region_id: u64,
        region_generation: u64,
        runtime_domain: u64,
        execution_id: u64,
        execution_slot: u64,
        execution_generation: u64,
        loan_id: u64,
        place: &'a str,
        kind: LoanKind,
        action: LoanAction,
    ) -> Result<Self, &'static str> {
        let candidate = Self {
            region_id,
            region_generation,
            runtime_domain,
            execution_id,
            execution_slot,
            execution_generation,
            loan_id,
            place,
            kind,
            action,
        };
        if !candidate.valid() {
            return Err("invalid state-region provenance");
        }
        Ok(candidate)
    }

    fn valid(&self) -> bool {
        (1..=MAX_JSON_SAFE_ID).contains(&self.region_id)
            && (1..=MAX_JSON_SAFE_ID).contains(&self.region_generation)
            && (1..=MAX_JSON_SAFE_ID).contains(&self.runtime_domain)
            && (1..=MAX_JSON_SAFE_ID).contains(&self.execution_id)
            && (1..=MAX_JSON_SAFE_ID).contains(&self.execution_slot)
            && (1..=MAX_JSON_SAFE_ID).contains(&self.execution_generation)
            && (1..=MAX_JSON_SAFE_ID).contains(&self.loan_id)
            && !self.place.trim().is_empty()
    }

    pub fn authorizes_release(&self, release: &Self) -> bool {
        self.valid()
            && release.valid()
            && self.action == LoanAction::Borrow
            && release.action == LoanAction::Release
            && self.region_id == release.region_id
            && self.region_generation == release.region_generation
            && self.runtime_domain == release.runtime_domain
            && self.execution_id == release.execution_id
            && self.execution_slot == release.execution_slot
            && self.execution_generation == release.execution_generation
            && self.loan_id == release.loan_id
            && self.place == release.place
            && self.kind == release.kind
    }

    pub fn release_record(&self) -> Result<Self, &'static str> {
        if !self.valid() || self.action != LoanAction::Borrow {
            return Err("only valid borrow provenance can derive release");
        }
        Ok(Self {
            action: LoanAction::Release,
            ..*self
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn borrow() -> StateRegionLoanProvenance<'static> {
        StateRegionLoanProvenance::new(
            9,
            3,
            7,
            1001,
            41,
            6,
            501,
            "inventory.stock",
            LoanKind::Write,
            LoanAction::Borrow,
        )
        .unwrap()
    }

    #[test]
    fn canonical_one_positive_and_nine_negative_releases() {
        let b = borrow();
        let good = b.release_record().unwrap();
        assert!(b.authorizes_release(&good));
        assert!(!good.authorizes_release(&b));
        assert!(!b.authorizes_release(&b));
        assert!(!good.authorizes_release(&good));
        assert!(good.release_record().is_err());

        let invalid = [
            StateRegionLoanProvenance {
                execution_id: 1002,
                ..good
            },
            StateRegionLoanProvenance {
                runtime_domain: 8,
                ..good
            },
            StateRegionLoanProvenance {
                execution_generation: 5,
                ..good
            },
            StateRegionLoanProvenance {
                execution_slot: 42,
                ..good
            },
            StateRegionLoanProvenance {
                region_id: 10,
                ..good
            },
            StateRegionLoanProvenance {
                region_generation: 2,
                ..good
            },
            StateRegionLoanProvenance {
                loan_id: 502,
                ..good
            },
            StateRegionLoanProvenance {
                place: "inventory.reserved",
                ..good
            },
            StateRegionLoanProvenance {
                kind: LoanKind::Read,
                ..good
            },
        ];
        assert_eq!(invalid.len(), 9);
        for (index, forged) in invalid.iter().enumerate() {
            assert!(
                !b.authorizes_release(forged),
                "forged release {index} accepted"
            );
        }
    }

    #[test]
    fn invalid_identity_and_default_like_values_never_authorize_release() {
        let b = borrow();
        for index in 0..7 {
            let mut invalid = b;
            match index {
                0 => invalid.region_id = 0,
                1 => invalid.region_generation = 0,
                2 => invalid.runtime_domain = 0,
                3 => invalid.execution_id = 0,
                4 => invalid.execution_slot = 0,
                5 => invalid.execution_generation = 0,
                _ => invalid.loan_id = 0,
            }
            assert!(!invalid.authorizes_release(&b.release_record().unwrap()));
            assert!(invalid.release_record().is_err());
        }
        assert!(
            StateRegionLoanProvenance::new(
                9,
                3,
                7,
                1001,
                41,
                6,
                501,
                " ",
                LoanKind::Read,
                LoanAction::Borrow,
            )
            .is_err()
        );
    }

    #[test]
    fn compiler_requirement_is_symbolic_and_fenced() {
        let b = StateRegionLoanRequirement::new(
            "inventory.state",
            true,
            501,
            "inventory.stock",
            LoanKind::Write,
            LoanAction::Borrow,
        )
        .unwrap();
        let release = StateRegionLoanRequirement {
            action: LoanAction::Release,
            ..b
        };
        assert!(b.authorizes_release(&release));
        for bad in [
            StateRegionLoanRequirement {
                region_binding: "other",
                ..release
            },
            StateRegionLoanRequirement {
                generation_fence: false,
                ..release
            },
            StateRegionLoanRequirement {
                loan_id: 502,
                ..release
            },
            StateRegionLoanRequirement {
                place: "inventory.reserved",
                ..release
            },
            StateRegionLoanRequirement {
                kind: LoanKind::Read,
                ..release
            },
            StateRegionLoanRequirement {
                action: LoanAction::Borrow,
                ..release
            },
        ] {
            assert!(!b.authorizes_release(&bad));
        }
        assert!(
            StateRegionLoanRequirement::new("", true, 1, "x", LoanKind::Read, LoanAction::Borrow)
                .is_err()
        );
        assert!(
            StateRegionLoanRequirement::new(
                "region",
                false,
                1,
                "x",
                LoanKind::Read,
                LoanAction::Borrow
            )
            .is_err()
        );
        assert!(
            StateRegionLoanRequirement::new(
                "region",
                true,
                0,
                "x",
                LoanKind::Read,
                LoanAction::Borrow
            )
            .is_err()
        );
    }

    #[test]
    fn wasm_json_identity_boundary_never_aliases_64_bit_values() {
        let max = MAX_JSON_SAFE_ID;
        let good = StateRegionLoanProvenance::new(
            max,
            max,
            max,
            max,
            max,
            max,
            max,
            "inventory.stock",
            LoanKind::Write,
            LoanAction::Borrow,
        )
        .unwrap();
        assert!(good.authorizes_release(&good.release_record().unwrap()));
        for component in 0..7 {
            let mut bad = good;
            match component {
                0 => bad.region_id = max + 1,
                1 => bad.region_generation = max + 1,
                2 => bad.runtime_domain = max + 1,
                3 => bad.execution_id = max + 1,
                4 => bad.execution_slot = max + 1,
                5 => bad.execution_generation = max + 1,
                _ => bad.loan_id = max + 1,
            }
            assert!(!bad.authorizes_release(&good.release_record().unwrap()));
            assert!(bad.release_record().is_err());
        }
        assert!(
            StateRegionLoanProvenance::new(
                max + 1,
                3,
                7,
                1001,
                41,
                6,
                501,
                "inventory.stock",
                LoanKind::Write,
                LoanAction::Borrow,
            )
            .is_err()
        );
        let requirement = StateRegionLoanRequirement::new(
            "inventory",
            true,
            max,
            "inventory.stock",
            LoanKind::Read,
            LoanAction::Borrow,
        )
        .unwrap();
        assert!(
            StateRegionLoanRequirement::new(
                "inventory",
                true,
                max + 1,
                "inventory.stock",
                LoanKind::Read,
                LoanAction::Borrow,
            )
            .is_err()
        );
        assert!(requirement.valid());
    }

    #[test]
    fn receipts_are_value_types_not_heap_backed_guard_objects() {
        assert!(!std::mem::needs_drop::<StateRegionLoanProvenance<'static>>());
        assert!(!std::mem::needs_drop::<StateRegionLoanRequirement<'static>>());
        assert!(std::mem::size_of::<StateRegionLoanProvenance<'static>>() <= 96);
        assert!(std::mem::size_of::<StateRegionLoanRequirement<'static>>() <= 64);
    }
}

// Host-only live admission. All operations require scheduler-authenticated
// identities; mutable access to this object serializes transitions.
// No compiler lowering, raw pointers, or physical turn reclamation is implied.
#[derive(Clone, Copy)]
struct ActiveLoan {
    execution_id: u64,
    execution_slot: u64,
    execution_generation: u64,
    loan_id: u64,
    kind: LoanKind,
}

impl ActiveLoan {
    fn matches(&self, r: &StateRegionLoanProvenance<'_>) -> bool {
        self.execution_id == r.execution_id
            && self.execution_slot == r.execution_slot
            && self.execution_generation == r.execution_generation
            && self.loan_id == r.loan_id
            && self.kind == r.kind
    }
}

pub struct StateRegionLoanLedger<const CAP: usize = 64> {
    region_id: u64,
    generation: u64,
    domain: u64,
    place: String,
    active: [Option<ActiveLoan>; CAP],
    last_id: u64,
    readers: usize,
    writer: bool,
    retired: bool,
}

impl<const CAP: usize> StateRegionLoanLedger<CAP> {
    pub fn new(region_id: u64, generation: u64, domain: u64, place: &str) -> Result<Self, &'static str> {
        if CAP == 0 || CAP > 64
            || ![region_id, generation, domain].iter().all(|id| (1..=MAX_JSON_SAFE_ID).contains(id))
            || place.trim().is_empty()
        {
            return Err("invalid state region identity or capacity");
        }
        Ok(Self {
            region_id, generation, domain, place: place.to_owned(),
            active: [None; CAP], last_id: 0, readers: 0, writer: false, retired: false,
        })
    }

    fn same_region(&self, r: &StateRegionLoanProvenance<'_>) -> bool {
        r.valid() && r.region_id == self.region_id && r.region_generation == self.generation
            && r.runtime_domain == self.domain && r.place == self.place.as_str()
    }

    pub fn try_borrow(&mut self, r: &StateRegionLoanProvenance<'_>) -> bool {
        if self.retired || !self.same_region(r) || r.action != LoanAction::Borrow
            || r.loan_id <= self.last_id || self.writer
            || (r.kind == LoanKind::Write && self.readers != 0)
        {
            return false;
        }
        if self.active.iter().flatten().any(|slot|
            slot.execution_id == r.execution_id || slot.execution_slot == r.execution_slot)
        {
            return false;
        }
        let Some(slot) = self.active.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(ActiveLoan {
            execution_id: r.execution_id,
            execution_slot: r.execution_slot,
            execution_generation: r.execution_generation,
            loan_id: r.loan_id,
            kind: r.kind,
        });
        self.last_id = r.loan_id;
        if r.kind == LoanKind::Write { self.writer = true; } else { self.readers += 1; }
        true
    }

    pub fn allows_access(&self, r: &StateRegionLoanProvenance<'_>, required: LoanKind) -> bool {
        !self.retired && self.same_region(r) && r.action == LoanAction::Borrow
            && (required == LoanKind::Read || r.kind == LoanKind::Write)
            && self.active.iter().flatten().any(|slot| slot.matches(r))
    }

    pub fn release(&mut self, r: &StateRegionLoanProvenance<'_>) -> bool {
        if !self.same_region(r) || r.action != LoanAction::Release { return false; }
        let Some(slot) = self.active.iter_mut().find(|slot| slot.as_ref().is_some_and(|loan| loan.matches(r))) else {
            return false;
        };
        *slot = None;
        if r.kind == LoanKind::Write { self.writer = false; } else { self.readers -= 1; }
        true
    }

    pub fn retire(&mut self) -> bool {
        if self.retired { return false; }
        self.retired = true;
        true
    }

    pub fn active_count(&self) -> usize { self.readers + usize::from(self.writer) }
    pub fn quiescent_after_retire(&self) -> bool { self.retired && self.active_count() == 0 }
}

// No state references escape. Overflow differs from access denial and is
// failure-neutral; a caller still has to release the live loan on unwind.
pub struct ProtectedScalarState<const CAP: usize = 64> {
    ledger: StateRegionLoanLedger<CAP>,
    value: i64,
}

impl<const CAP: usize> ProtectedScalarState<CAP> {
    pub fn new(region_id: u64, generation: u64, domain: u64, place: &str, initial: i64) -> Result<Self, &'static str> {
        Ok(Self { ledger: StateRegionLoanLedger::new(region_id, generation, domain, place)?, value: initial })
    }
    pub fn try_borrow(&mut self, r: &StateRegionLoanProvenance<'_>) -> bool { self.ledger.try_borrow(r) }
    pub fn release(&mut self, r: &StateRegionLoanProvenance<'_>) -> bool { self.ledger.release(r) }
    pub fn read(&self, r: &StateRegionLoanProvenance<'_>) -> Option<i64> {
        self.ledger.allows_access(r, LoanKind::Read).then_some(self.value)
    }
    pub fn write(&mut self, r: &StateRegionLoanProvenance<'_>, value: i64) -> bool {
        if !self.ledger.allows_access(r, LoanKind::Write) { return false; }
        self.value = value;
        true
    }
    pub fn add_checked(&mut self, r: &StateRegionLoanProvenance<'_>, delta: i64) -> Result<Option<i64>, &'static str> {
        if !self.ledger.allows_access(r, LoanKind::Write) { return Ok(None); }
        let next = self.value.checked_add(delta).ok_or("protected scalar addition overflow")?;
        self.value = next;
        Ok(Some(next))
    }
    pub fn retire(&mut self) -> bool { self.ledger.retire() }
    pub fn quiescent_after_retire(&self) -> bool { self.ledger.quiescent_after_retire() }
}

#[cfg(test)]
mod live_ledger_tests {
    use super::*;

    fn claim(loan: u64, execution: u64, kind: LoanKind) -> StateRegionLoanProvenance<'static> {
        StateRegionLoanProvenance::new(1, 1, 7, execution, execution, 1, loan, "count", kind, LoanAction::Borrow).unwrap()
    }

    #[test]
    fn live_shared_and_exclusive_admission() {
        let mut g = StateRegionLoanLedger::<2>::new(1, 1, 7, "count").unwrap();
        let a = claim(1, 1, LoanKind::Read);
        let b = claim(2, 2, LoanKind::Read);
        let w = claim(3, 3, LoanKind::Write);
        assert!(g.try_borrow(&a)); assert!(g.try_borrow(&b));
        assert_eq!(g.active_count(), 2);
        assert!(!g.try_borrow(&claim(3, 3, LoanKind::Read)));
        assert!(!g.try_borrow(&w));
        assert!(g.allows_access(&a, LoanKind::Read)); assert!(!g.allows_access(&a, LoanKind::Write));
        assert!(g.release(&a.release_record().unwrap())); assert!(!g.try_borrow(&w));
        assert!(g.release(&b.release_record().unwrap())); assert!(g.try_borrow(&w));
        assert!(g.allows_access(&w, LoanKind::Read)); assert!(g.allows_access(&w, LoanKind::Write));
        assert!(!g.try_borrow(&claim(4, 4, LoanKind::Read)));
        assert!(g.release(&w.release_record().unwrap()));
        assert_eq!(g.active_count(), 0);
        assert!(!g.try_borrow(&w)); assert!(!g.release(&w.release_record().unwrap()));
    }

    #[test]
    fn live_exact_provenance_and_no_reentry() {
        let mut g = StateRegionLoanLedger::<64>::new(1, 1, 7, "count").unwrap();
        let a = claim(1, 1, LoanKind::Read);
        assert!(g.try_borrow(&a));
        assert!(!g.try_borrow(&claim(2, 1, LoanKind::Read)));
        let replaced = StateRegionLoanProvenance { execution_slot: 1, execution_generation: 2, ..claim(2, 2, LoanKind::Read) };
        assert!(!g.try_borrow(&replaced));
        for bad in [
            StateRegionLoanProvenance { region_id: 2, ..a },
            StateRegionLoanProvenance { region_generation: 2, ..a },
            StateRegionLoanProvenance { runtime_domain: 8, ..a },
            StateRegionLoanProvenance { execution_id: 2, ..a },
            StateRegionLoanProvenance { execution_slot: 2, ..a },
            StateRegionLoanProvenance { execution_generation: 2, ..a },
            StateRegionLoanProvenance { loan_id: 2, ..a },
            StateRegionLoanProvenance { place: "other", ..a },
            StateRegionLoanProvenance { kind: LoanKind::Write, ..a },
        ] {
            assert!(!g.allows_access(&bad, LoanKind::Read));
            assert!(!g.release(&bad.release_record().unwrap()));
            assert_eq!(g.active_count(), 1);
        }
        assert!(g.release(&a.release_record().unwrap()));
        assert!(!g.allows_access(&a, LoanKind::Read)); assert!(!g.try_borrow(&a));
        assert!(g.try_borrow(&replaced)); assert!(g.release(&replaced.release_record().unwrap()));
    }

    #[test]
    fn live_retirement_and_owned_place() {
        let mut g = StateRegionLoanLedger::<64>::new(1, 1, 7, "count").unwrap();
        let mut peer = StateRegionLoanLedger::<64>::new(1, 2, 7, "count").unwrap();
        let a = claim(1, 1, LoanKind::Read);
        let b = StateRegionLoanProvenance { region_generation: 2, ..a };
        {
            let temporary = String::from("count");
            let transient = StateRegionLoanProvenance { place: temporary.as_str(), ..a };
            assert!(g.try_borrow(&transient));
        }
        assert!(peer.try_borrow(&b)); assert!(!g.release(&b.release_record().unwrap()));
        assert!(g.allows_access(&a, LoanKind::Read));
        assert!(g.retire()); assert!(!g.retire()); assert!(!g.quiescent_after_retire());
        assert!(!g.allows_access(&a, LoanKind::Read)); assert!(!g.try_borrow(&claim(2, 2, LoanKind::Read)));
        assert!(g.release(&a.release_record().unwrap())); assert!(g.quiescent_after_retire());
        assert!(!g.release(&a.release_record().unwrap()));
        assert!(peer.allows_access(&b, LoanKind::Read));
        assert!(peer.release(&b.release_record().unwrap())); assert!(peer.retire()); assert!(peer.quiescent_after_retire());
    }

    #[test]
    fn live_identity_exhaustion_and_invalid_bounds() {
        assert!(StateRegionLoanLedger::<0>::new(1, 1, 7, "count").is_err());
        assert!(StateRegionLoanLedger::<65>::new(1, 1, 7, "count").is_err());
        assert!(StateRegionLoanLedger::<64>::new(MAX_JSON_SAFE_ID + 1, 1, 7, "count").is_err());
        let mut g = StateRegionLoanLedger::<64>::new(1, 1, 7, "count").unwrap();
        let a = claim(1, 1, LoanKind::Read);
        assert!(!g.try_borrow(&a.release_record().unwrap()));
        let last = claim(MAX_JSON_SAFE_ID, 1, LoanKind::Write);
        assert!(g.try_borrow(&last)); assert!(g.release(&last.release_record().unwrap()));
        assert!(!g.try_borrow(&a)); assert!(!g.try_borrow(&last));
        assert!(g.retire()); assert!(g.quiescent_after_retire());
    }

    #[test]
    fn protected_scalar_permissions_and_isolation() {
        let mut a = ProtectedScalarState::<2>::new(1, 1, 7, "count", 40).unwrap();
        let mut b = ProtectedScalarState::<2>::new(1, 2, 7, "count", -3).unwrap();
        let r = claim(1, 1, LoanKind::Read);
        let w = claim(2, 2, LoanKind::Write);
        let foreign = StateRegionLoanProvenance { region_generation: 2, ..w };
        assert_eq!(a.read(&r), None); assert!(a.try_borrow(&r)); assert_eq!(a.read(&r), Some(40));
        assert!(!a.write(&r, 99)); assert_eq!(a.add_checked(&r, 1), Ok(None));
        assert!(!a.try_borrow(&w)); assert!(b.try_borrow(&foreign));
        assert_eq!(a.read(&foreign), None); assert!(!a.write(&foreign, 9));
        assert!(a.release(&r.release_record().unwrap())); assert!(a.try_borrow(&w));
        assert_eq!(a.add_checked(&w, 2), Ok(Some(42))); assert_eq!(a.read(&w), Some(42));
        assert_eq!(b.read(&foreign), Some(-3)); assert_eq!(a.read(&r), None);
        assert!(a.release(&w.release_record().unwrap())); assert!(b.release(&foreign.release_record().unwrap()));
    }

    #[test]
    fn protected_scalar_checked_boundaries() {
        let mut a = ProtectedScalarState::<64>::new(1, 1, 7, "count", 0).unwrap();
        let w = claim(1, 1, LoanKind::Write);
        assert!(a.try_borrow(&w));
        for initial in [i64::MIN, i64::MAX] {
            assert!(a.write(&w, initial));
            assert!(a.add_checked(&w, if initial == i64::MIN { -1 } else { 1 }).is_err());
            assert_eq!(a.read(&w), Some(initial));
        }
        assert!(a.write(&w, 0));
        assert_eq!(a.add_checked(&w, i64::MIN), Ok(Some(i64::MIN)));
        assert_eq!(a.add_checked(&w, i64::MAX), Ok(Some(-1)));
        assert_eq!(a.add_checked(&w, 1), Ok(Some(0)));
        assert!(a.release(&w.release_record().unwrap()));
    }

    #[test]
    fn protected_scalar_retirement_preserves_committed_state_until_close() {
        let mut a = ProtectedScalarState::<64>::new(1, 1, 7, "count", 0).unwrap();
        let w = claim(1, 1, LoanKind::Write);
        let r = claim(2, 1, LoanKind::Read);
        assert!(a.try_borrow(&w)); assert!(a.write(&w, 42)); assert!(a.release(&w.release_record().unwrap()));
        assert!(a.try_borrow(&r)); assert_eq!(a.read(&r), Some(42));
        assert!(a.retire()); assert!(!a.retire()); assert!(!a.quiescent_after_retire());
        assert_eq!(a.read(&r), None); assert!(!a.write(&w, 9)); assert!(!a.try_borrow(&claim(3, 3, LoanKind::Read)));
        assert!(a.release(&r.release_record().unwrap())); assert!(a.quiescent_after_retire());
        assert!(!a.release(&r.release_record().unwrap()));
    }
}
