//! Host-owned scalar-region prototype for future compiled actor arenas.
//!
//! Never exposes a raw guest pointer or lets a guest fabricate an authority.
//! The owner controls one immutable shape and fixed capacity. This is a Rust
//! runtime *foundation*, not AttoLang actor lowering or guest zero-GC proof.

use crate::layout::{LayoutError, LayoutPlan, ScalarField};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct RegionLease {
    authority: Arc<()>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionError {
    RegionRetired,
    ForeignOrStaleLease,
    AllocationBudget,
    AllocationFailure,
    WrongScalarType,
    NoncanonicalBoolean,
    Layout(LayoutError),
}

impl From<LayoutError> for RegionError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

/// One owner for a fixed-capacity scalar allocation. No tracing collector,
/// guest-managed heap, resizing, direct pointers or interior mutable aliases.
pub struct OwnedScalarRegion {
    plan: LayoutPlan,
    bytes: Vec<u8>,
    authority: Arc<()>,
    generation: u64,
    retired: bool,
}

impl OwnedScalarRegion {
    /// The quota is enforced even if a compromised caller tries to pass a
    /// previously constructed plan without the intended actor budget.
    pub fn new(plan: LayoutPlan, quota_bytes: u64) -> Result<Self, RegionError> {
        if plan.allocation_bytes() > quota_bytes {
            return Err(RegionError::AllocationBudget);
        }
        let length =
            usize::try_from(plan.allocation_bytes()).map_err(|_| RegionError::AllocationBudget)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| RegionError::AllocationFailure)?;
        bytes.resize(length, 0);
        Ok(Self {
            plan,
            bytes,
            authority: Arc::new(()),
            generation: 1,
            retired: false,
        })
    }

    pub fn lease(&self) -> Result<RegionLease, RegionError> {
        if self.retired {
            return Err(RegionError::RegionRetired);
        }
        Ok(RegionLease {
            authority: Arc::clone(&self.authority),
            generation: self.generation,
        })
    }

    pub fn allocation_bytes(&self) -> u64 {
        if self.retired {
            0
        } else {
            self.plan.allocation_bytes()
        }
    }

    fn admit(&self, lease: &RegionLease) -> Result<(), RegionError> {
        if self.retired {
            return Err(RegionError::RegionRetired);
        }
        if self.generation != lease.generation
            || !Arc::ptr_eq(&self.authority, &lease.authority)
        {
            return Err(RegionError::ForeignOrStaleLease);
        }
        Ok(())
    }

    fn typed_span(
        &self,
        field: usize,
        index: u64,
        kind: ScalarField,
    ) -> Result<std::ops::Range<usize>, RegionError> {
        let placement = self
            .plan
            .fields()
            .get(field)
            .ok_or(LayoutError::UnknownField)?;
        if placement.kind != kind {
            return Err(RegionError::WrongScalarType);
        }
        let start = self.plan.field_offset(field, index)?;
        let start = usize::try_from(start).map_err(|_| RegionError::AllocationBudget)?;
        let width = usize::try_from(kind.bytes()).map_err(|_| RegionError::AllocationBudget)?;
        let end = start.checked_add(width).ok_or(RegionError::AllocationBudget)?;
        if end > self.bytes.len() {
            return Err(RegionError::AllocationBudget);
        }
        Ok(start..end)
    }

    fn read_eight(
        &self,
        lease: &RegionLease,
        field: usize,
        index: u64,
        kind: ScalarField,
    ) -> Result<[u8; 8], RegionError> {
        self.admit(lease)?;
        let span = self.typed_span(field, index, kind)?;
        self.bytes
            .get(span)
            .ok_or(RegionError::AllocationBudget)?
            .try_into()
            .map_err(|_| RegionError::AllocationBudget)
    }

    fn write_eight(
        &mut self,
        lease: &RegionLease,
        field: usize,
        index: u64,
        kind: ScalarField,
        value: [u8; 8],
    ) -> Result<(), RegionError> {
        self.admit(lease)?;
        let span = self.typed_span(field, index, kind)?;
        self.bytes[span].copy_from_slice(&value);
        Ok(())
    }

    /// This bridge is safe byte access, not an unaligned native pointer cast.
    /// The little-endian representation is stable across host architectures.
    pub fn read_i64(
        &self,
        lease: &RegionLease,
        field: usize,
        index: u64,
    ) -> Result<i64, RegionError> {
        Ok(i64::from_le_bytes(self.read_eight(lease, field, index, ScalarField::I64)?))
    }

    pub fn write_i64(
        &mut self,
        lease: &RegionLease,
        field: usize,
        index: u64,
        value: i64,
    ) -> Result<(), RegionError> {
        self.write_eight(lease, field, index, ScalarField::I64, value.to_le_bytes())
    }

    /// Preserve *exact* IEEE-754 bits, including negative zero and NaN payloads.
    pub fn read_f64(
        &self,
        lease: &RegionLease,
        field: usize,
        index: u64,
    ) -> Result<f64, RegionError> {
        let bytes = self.read_eight(lease, field, index, ScalarField::F64)?;
        Ok(f64::from_bits(u64::from_le_bytes(bytes)))
    }

    pub fn write_f64(
        &mut self,
        lease: &RegionLease,
        field: usize,
        index: u64,
        value: f64,
    ) -> Result<(), RegionError> {
        self.write_eight(lease, field, index, ScalarField::F64, value.to_bits().to_le_bytes())
    }

    /// Boolean values are encoded as canonical 0 and 1, not host bool ABI.
    pub fn read_bool(
        &self,
        lease: &RegionLease,
        field: usize,
        index: u64,
    ) -> Result<bool, RegionError> {
        self.admit(lease)?;
        let span = self.typed_span(field, index, ScalarField::Bool)?;
        match self.bytes[span.start] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(RegionError::NoncanonicalBoolean),
        }
    }

    pub fn write_bool(
        &mut self,
        lease: &RegionLease,
        field: usize,
        index: u64,
        value: bool,
    ) -> Result<(), RegionError> {
        self.admit(lease)?;
        let span = self.typed_span(field, index, ScalarField::Bool)?;
        self.bytes[span.start] = u8::from(value);
        Ok(())
    }

    /// Prevents new admission and immediately drops the old byte allocation.
    /// The owner must first prove no generated native guest calls can access
    /// this region. No physical memory safety is inferred from a lease alone.
    pub fn retire(&mut self) -> Result<(), RegionError> {
        if self.retired {
            return Err(RegionError::RegionRetired);
        }
        // Logical clearing only; not a secure cryptographic memory wipe.
        self.bytes.fill(0);
        self.bytes = Vec::new();
        // Retired generations are never reused by this region.
        self.generation += 1;
        self.retired = true;
        Ok(())
    }
}

impl Drop for OwnedScalarRegion {
    fn drop(&mut self) {
        // Logical best-effort clearing; Rust frees the allocation on drop.
        // This is not guaranteed volatile zeroization of secret data.
        self.bytes.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{plan_layout, AddressWidth, LayoutMode};

    fn region(mode: LayoutMode) -> OwnedScalarRegion {
        let plan = plan_layout(
            &[ScalarField::I64, ScalarField::U8, ScalarField::I64],
            3,
            mode,
            AddressWidth::Bits32,
            80,
        )
        .unwrap();
        OwnedScalarRegion::new(plan, 80).unwrap()
    }

    #[test]
    fn fixed_capacity_scalar_roundtrips_in_both_layouts() {
        for mode in [LayoutMode::Aos, LayoutMode::Soa] {
            let mut owned = region(mode);
            let lease = owned.lease().unwrap();
            assert_eq!(owned.read_i64(&lease, 0, 0), Ok(0));
            for index in 0..3 {
                owned.write_i64(&lease, 0, index, i64::MIN + index as i64).unwrap();
                owned.write_i64(&lease, 2, index, i64::MAX - index as i64).unwrap();
            }
            for index in 0..3 {
                assert_eq!(owned.read_i64(&lease, 0, index), Ok(i64::MIN + index as i64));
                assert_eq!(owned.read_i64(&lease, 2, index), Ok(i64::MAX - index as i64));
            }
            assert_eq!(owned.read_i64(&lease, 1, 0), Err(RegionError::WrongScalarType));
            assert_eq!(owned.write_i64(&lease, 1, 0, 42), Err(RegionError::WrongScalarType));
            assert_eq!(
                owned.write_i64(&lease, 0, 3, 42),
                Err(RegionError::Layout(LayoutError::UnknownElement))
            );
            assert_eq!(
                owned.read_i64(&lease, 3, 0),
                Err(RegionError::Layout(LayoutError::UnknownField))
            );
        }
    }

    #[test]
    fn canonical_bool_and_ieee754_bits_roundtrip_without_float_arithmetic() {
        let shape = [ScalarField::Bool, ScalarField::I64, ScalarField::F64];
        let expected_nan = f64::from_bits(0x7ff8_0000_0000_0123);
        for mode in [LayoutMode::Aos, LayoutMode::Soa] {
            let plan = plan_layout(&shape, 3, mode, AddressWidth::Bits32, 80).unwrap();
            let mut owned = OwnedScalarRegion::new(plan, 80).unwrap();
            let lease = owned.lease().unwrap();
            assert_eq!(owned.read_bool(&lease, 0, 0), Ok(false));
            owned.write_bool(&lease, 0, 0, true).unwrap();
            owned.write_bool(&lease, 0, 1, false).unwrap();
            owned.write_f64(&lease, 2, 0, -0.0).unwrap();
            owned.write_f64(&lease, 2, 1, expected_nan).unwrap();
            owned.write_f64(&lease, 2, 2, f64::INFINITY).unwrap();
            assert_eq!(owned.read_bool(&lease, 0, 0), Ok(true));
            assert_eq!(owned.read_bool(&lease, 0, 1), Ok(false));
            assert_eq!(owned.read_f64(&lease, 2, 0).unwrap().to_bits(), (-0.0_f64).to_bits());
            assert_eq!(
                owned.read_f64(&lease, 2, 1).unwrap().to_bits(),
                expected_nan.to_bits()
            );
            assert_eq!(
                owned.read_f64(&lease, 2, 2).unwrap().to_bits(),
                f64::INFINITY.to_bits()
            );
            assert_eq!(
                owned.read_i64(&lease, 2, 0),
                Err(RegionError::WrongScalarType)
            );
            assert_eq!(
                owned.write_bool(&lease, 1, 0, true),
                Err(RegionError::WrongScalarType)
            );
        }
    }

    #[test]
    fn forged_cross_region_authority_is_rejected() {
        let mut left = region(LayoutMode::Aos);
        let mut right = region(LayoutMode::Aos);
        let left_lease = left.lease().unwrap();
        let right_lease = right.lease().unwrap();
        left.write_i64(&left_lease, 0, 0, 17).unwrap();
        right.write_i64(&right_lease, 0, 0, 29).unwrap();
        assert_eq!(
            left.read_i64(&right_lease, 0, 0),
            Err(RegionError::ForeignOrStaleLease)
        );
        assert_eq!(
            right.write_i64(&left_lease, 0, 0, 33),
            Err(RegionError::ForeignOrStaleLease)
        );
        assert_eq!(left.read_i64(&left_lease, 0, 0), Ok(17));
        assert_eq!(right.read_i64(&right_lease, 0, 0), Ok(29));
    }

    #[test]
    fn retirement_revokes_all_existing_leases_and_disposes_allocation() {
        let mut owned = region(LayoutMode::Soa);
        let first = owned.lease().unwrap();
        let cloned = first.clone();
        owned.write_i64(&first, 0, 1, 42).unwrap();
        assert!(owned.allocation_bytes() > 0);
        owned.retire().unwrap();
        assert_eq!(owned.allocation_bytes(), 0);
        assert_eq!(owned.lease().unwrap_err(), RegionError::RegionRetired);
        assert_eq!(owned.read_i64(&first, 0, 1), Err(RegionError::RegionRetired));
        assert_eq!(owned.write_i64(&cloned, 0, 1, 99), Err(RegionError::RegionRetired));
        assert_eq!(owned.retire(), Err(RegionError::RegionRetired));
    }

    #[test]
    fn independent_owner_does_not_revive_a_retired_lease() {
        let mut original = region(LayoutMode::Aos);
        let expired = original.lease().unwrap();
        original.retire().unwrap();
        let mut replacement = region(LayoutMode::Aos);
        assert_eq!(
            replacement.write_i64(&expired, 0, 0, 42),
            Err(RegionError::ForeignOrStaleLease)
        );
        let current = replacement.lease().unwrap();
        replacement.write_i64(&current, 0, 0, 42).unwrap();
        assert_eq!(replacement.read_i64(&current, 0, 0), Ok(42));
    }

    #[test]
    fn fixed_quota_is_checked_again_after_layout_planning() {
        for mode in [LayoutMode::Aos, LayoutMode::Soa] {
            let plan = plan_layout(
                &[ScalarField::I64, ScalarField::I64],
                3,
                mode,
                AddressWidth::Bits32,
                128,
            )
            .unwrap();
            assert_eq!(plan.allocation_bytes(), 48);
            assert!(matches!(
                OwnedScalarRegion::new(plan, 47),
                Err(RegionError::AllocationBudget)
            ));
        }
    }

    #[test]
    fn empty_capacity_has_no_live_index() {
        for mode in [LayoutMode::Aos, LayoutMode::Soa] {
            let plan = plan_layout(
                &[ScalarField::I64],
                0,
                mode,
                AddressWidth::Bits32,
                0,
            )
            .unwrap();
            let owned = OwnedScalarRegion::new(plan, 0).unwrap();
            let lease = owned.lease().unwrap();
            assert_eq!(owned.allocation_bytes(), 0);
            assert_eq!(
                owned.read_i64(&lease, 0, 0),
                Err(RegionError::Layout(LayoutError::UnknownElement))
            );
        }
    }
}
