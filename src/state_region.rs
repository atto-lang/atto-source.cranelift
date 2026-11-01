//! State-region provenance v1: *receipt values*, never guest-visible authority.
//!
//! Compiler requirements name symbolic regions and current logical execution;
//! only the trusted runtime may bind concrete region and scheduler generations.
//! These POD values contain no heap storage, locks, handles, or mutable access.
//! NO compiler lowering or state-container execution is implied by this module.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanKind { Read, Write }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanAction { Borrow, Release }

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
        if region_binding.trim().is_empty() { return Err("region binding required"); }
        if !generation_fence { return Err("state loan must be generation fenced"); }
        if loan_id == 0 { return Err("loan id must be nonzero"); }
        if place.trim().is_empty() { return Err("state loan place required"); }
        Ok(Self { region_binding, generation_fence, loan_id, place, kind, action })
    }

    pub fn authorizes_release(&self, release: &Self) -> bool {
        self.valid() && release.valid()
            && self.action == LoanAction::Borrow
            && release.action == LoanAction::Release
            && self.region_binding == release.region_binding
            && self.generation_fence == release.generation_fence
            && self.loan_id == release.loan_id && self.place == release.place
            && self.kind == release.kind
    }

    fn valid(&self) -> bool {
        !self.region_binding.trim().is_empty() && self.generation_fence
            && self.loan_id != 0 && !self.place.trim().is_empty()
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
            region_id, region_generation, runtime_domain,
            execution_id, execution_slot, execution_generation,
            loan_id, place, kind, action,
        };
        if !candidate.valid() { return Err("invalid state-region provenance"); }
        Ok(candidate)
    }

    fn valid(&self) -> bool {
        self.region_id != 0 && self.region_generation != 0
            && self.runtime_domain != 0 && self.execution_id != 0
            && self.execution_slot != 0 && self.execution_generation != 0
            && self.loan_id != 0 && !self.place.trim().is_empty()
    }

    pub fn authorizes_release(&self, release: &Self) -> bool {
        self.valid() && release.valid()
            && self.action == LoanAction::Borrow
            && release.action == LoanAction::Release
            && self.region_id == release.region_id
            && self.region_generation == release.region_generation
            && self.runtime_domain == release.runtime_domain
            && self.execution_id == release.execution_id
            && self.execution_slot == release.execution_slot
            && self.execution_generation == release.execution_generation
            && self.loan_id == release.loan_id && self.place == release.place
            && self.kind == release.kind
    }

    pub fn release_record(&self) -> Result<Self, &'static str> {
        if !self.valid() || self.action != LoanAction::Borrow {
            return Err("only valid borrow provenance can derive release");
        }
        Ok(Self { action: LoanAction::Release, ..*self })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn borrow() -> StateRegionLoanProvenance<'static> {
        StateRegionLoanProvenance::new(
            9, 3, 7, 1001, 41, 6, 501, "inventory.stock",
            LoanKind::Write, LoanAction::Borrow,
        ).unwrap()
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
            StateRegionLoanProvenance { execution_id: 1002, ..good },
            StateRegionLoanProvenance { runtime_domain: 8, ..good },
            StateRegionLoanProvenance { execution_generation: 5, ..good },
            StateRegionLoanProvenance { execution_slot: 42, ..good },
            StateRegionLoanProvenance { region_id: 10, ..good },
            StateRegionLoanProvenance { region_generation: 2, ..good },
            StateRegionLoanProvenance { loan_id: 502, ..good },
            StateRegionLoanProvenance { place: "inventory.reserved", ..good },
            StateRegionLoanProvenance { kind: LoanKind::Read, ..good },
        ];
        assert_eq!(invalid.len(), 9);
        for (index, forged) in invalid.iter().enumerate() {
            assert!(!b.authorizes_release(forged), "forged release {index} accepted");
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
        assert!(StateRegionLoanProvenance::new(
            9, 3, 7, 1001, 41, 6, 501, " ",
            LoanKind::Read, LoanAction::Borrow,
        ).is_err());
    }

    #[test]
    fn compiler_requirement_is_symbolic_and_fenced() {
        let b = StateRegionLoanRequirement::new(
            "inventory.state", true, 501, "inventory.stock",
            LoanKind::Write, LoanAction::Borrow,
        ).unwrap();
        let release = StateRegionLoanRequirement {
            action: LoanAction::Release, ..b
        };
        assert!(b.authorizes_release(&release));
        for bad in [
            StateRegionLoanRequirement { region_binding: "other", ..release },
            StateRegionLoanRequirement { generation_fence: false, ..release },
            StateRegionLoanRequirement { loan_id: 502, ..release },
            StateRegionLoanRequirement { place: "inventory.reserved", ..release },
            StateRegionLoanRequirement { kind: LoanKind::Read, ..release },
            StateRegionLoanRequirement { action: LoanAction::Borrow, ..release },
        ] {
            assert!(!b.authorizes_release(&bad));
        }
        assert!(StateRegionLoanRequirement::new(
            "", true, 1, "x", LoanKind::Read, LoanAction::Borrow
        ).is_err());
        assert!(StateRegionLoanRequirement::new(
            "region", false, 1, "x", LoanKind::Read, LoanAction::Borrow
        ).is_err());
        assert!(StateRegionLoanRequirement::new(
            "region", true, 0, "x", LoanKind::Read, LoanAction::Borrow
        ).is_err());
    }

    #[test]
    fn receipts_are_value_types_not_heap_backed_guard_objects() {
        assert!(!std::mem::needs_drop::<StateRegionLoanProvenance<'static>>());
        assert!(!std::mem::needs_drop::<StateRegionLoanRequirement<'static>>());
        assert!(std::mem::size_of::<StateRegionLoanProvenance<'static>>() <= 96);
        assert!(std::mem::size_of::<StateRegionLoanRequirement<'static>>() <= 64);
    }
}
