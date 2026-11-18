//! Compile-time, fixed-capacity AoS/SoA planning for future owned guest aggregates.
//!
//! This does not allocate guest memory, trace objects, lower classes, or enable
//! untrusted native JIT. A runtime allocator must separately prove ownership,
//! base alignment, deterministic teardown, and generation-safe references.

/// These are plain scalar storage widths, not source-language type identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarField {
    /// Canonical one-byte 0/1 boolean in guest storage.
    Bool,
    U8,
    U16,
    U32,
    U64,
    I64,
    F32,
    F64,
}

impl ScalarField {
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Bool | Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
        }
    }

    pub const fn alignment(self) -> u64 {
        self.bytes()
    }
}

/// An AoS object has adjacent fields; a SoA (inverted class) has field columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Aos,
    Soa,
}

/// Address bounds must be checked for the *target*, not the compiler host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressWidth {
    Bits32,
    Bits64,
}

impl AddressWidth {
    const fn largest_offset(self) -> u64 {
        match self {
            Self::Bits32 => u32::MAX as u64,
            Self::Bits64 => u64::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldPlacement {
    pub kind: ScalarField,
    pub offset: u64,
    pub element_stride: u64,
    pub bytes: u64,
    pub alignment: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutPlan {
    mode: LayoutMode,
    capacity: u64,
    allocation_alignment: u64,
    allocation_bytes: u64,
    /// Only meaningful for AoS; SoA elements have individual field strides.
    record_stride: Option<u64>,
    fields: Vec<FieldPlacement>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    EmptyShape,
    TooManyFields,
    ArithmeticOverflow,
    BudgetExceeded,
    UnknownField,
    UnknownElement,
}

const MAX_FIELDS: usize = 1024;

fn align_up(value: u64, alignment: u64) -> Result<u64, LayoutError> {
    // Every admitted scalar field has a nonzero power-of-two alignment.
    let mask = alignment - 1;
    value
        .checked_add(mask)
        .map(|next| next & !mask)
        .ok_or(LayoutError::ArithmeticOverflow)
}

/// Calculates a deterministic layout without allocating any guest memory.
///
/// `max_bytes` is the caller's actor/region quota; target pointer width is
/// enforced independently. The caller must allocate an appropriately aligned
/// base and keep it live for the entire use of any returned offset.
pub fn plan_layout(
    fields: &[ScalarField],
    capacity: u64,
    mode: LayoutMode,
    width: AddressWidth,
    max_bytes: u64,
) -> Result<LayoutPlan, LayoutError> {
    if fields.is_empty() {
        return Err(LayoutError::EmptyShape);
    }
    if fields.len() > MAX_FIELDS {
        return Err(LayoutError::TooManyFields);
    }

    let allocation_alignment = fields.iter().map(|field| field.alignment()).max().unwrap_or(1);
    let mut placements = Vec::with_capacity(fields.len());

    let (allocation_bytes, record_stride) = match mode {
        LayoutMode::Aos => {
            let mut cursor = 0_u64;
            for field in fields {
                cursor = align_up(cursor, field.alignment())?;
                placements.push(FieldPlacement {
                    kind: *field,
                    offset: cursor,
                    element_stride: 0,
                    bytes: field.bytes(),
                    alignment: field.alignment(),
                });
                cursor = cursor
                    .checked_add(field.bytes())
                    .ok_or(LayoutError::ArithmeticOverflow)?;
            }
            let stride = align_up(cursor, allocation_alignment)?;
            for placement in &mut placements {
                placement.element_stride = stride;
            }
            let length = stride
                .checked_mul(capacity)
                .ok_or(LayoutError::ArithmeticOverflow)?;
            (length, Some(stride))
        }
        LayoutMode::Soa => {
            let mut cursor = 0_u64;
            for field in fields {
                cursor = align_up(cursor, field.alignment())?;
                placements.push(FieldPlacement {
                    kind: *field,
                    offset: cursor,
                    element_stride: field.bytes(),
                    bytes: field.bytes(),
                    alignment: field.alignment(),
                });
                let column_bytes = field
                    .bytes()
                    .checked_mul(capacity)
                    .ok_or(LayoutError::ArithmeticOverflow)?;
                cursor = cursor
                    .checked_add(column_bytes)
                    .ok_or(LayoutError::ArithmeticOverflow)?;
            }
            (align_up(cursor, allocation_alignment)?, None)
        }
    };

    if allocation_bytes > max_bytes || allocation_bytes > width.largest_offset() {
        return Err(LayoutError::BudgetExceeded);
    }
    Ok(LayoutPlan {
        mode,
        capacity,
        allocation_alignment,
        allocation_bytes,
        record_stride,
        fields: placements,
    })
}

impl LayoutPlan {
    pub fn mode(&self) -> LayoutMode {
        self.mode
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn allocation_alignment(&self) -> u64 {
        self.allocation_alignment
    }

    pub fn allocation_bytes(&self) -> u64 {
        self.allocation_bytes
    }

    pub fn record_stride(&self) -> Option<u64> {
        self.record_stride
    }

    pub fn fields(&self) -> &[FieldPlacement] {
        &self.fields
    }

    /// Returns a checked byte offset. It never produces a pointer, borrow,
    /// reference, or lifetime claim. Caller must separately validate its arena.
    pub fn field_offset(&self, field: usize, element: u64) -> Result<u64, LayoutError> {
        let placement = self.fields.get(field).ok_or(LayoutError::UnknownField)?;
        if element >= self.capacity {
            return Err(LayoutError::UnknownElement);
        }
        let displacement = placement
            .element_stride
            .checked_mul(element)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let offset = placement
            .offset
            .checked_add(displacement)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let end = offset
            .checked_add(placement.bytes)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        if end > self.allocation_bytes {
            return Err(LayoutError::BudgetExceeded);
        }
        Ok(offset)
    }
}

/// Static feedback for a *future compile*, not an in-place runtime layout
/// migration. JIT tracing must not rewrite active owned allocations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessCounts {
    pub object_major: u64,
    pub field_major: u64,
}

impl AccessCounts {
    /// Advisory for an explicitly declared automatic store only.
    pub fn suggested_layout(self) -> LayoutMode {
        if self.field_major > self.object_major {
            LayoutMode::Soa
        } else {
            LayoutMode::Aos
        }
    }
}

/// A typed, compile-time declaration decides whether hints are permitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutIntent {
    OrdinaryArray,
    InvertedArray,
    ExplicitAos,
    ExplicitSoa,
    Auto,
}

impl LayoutIntent {
    /// Explicit source layouts cannot be changed by profile feedback.
    pub fn resolve(self, samples: AccessCounts) -> LayoutMode {
        match self {
            Self::OrdinaryArray | Self::ExplicitAos => LayoutMode::Aos,
            Self::InvertedArray | Self::ExplicitSoa => LayoutMode::Soa,
            Self::Auto => samples.suggested_layout(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPE: [ScalarField; 3] = [ScalarField::U8, ScalarField::I64, ScalarField::U16];

    #[test]
    fn aos_layout_respects_alignment_and_bounded_offsets() {
        let plan = plan_layout(&SHAPE, 3, LayoutMode::Aos, AddressWidth::Bits64, 72).unwrap();
        assert_eq!(plan.record_stride, Some(24));
        assert_eq!(plan.allocation_alignment, 8);
        assert_eq!(plan.allocation_bytes, 72);
        assert_eq!(plan.field_offset(0, 0), Ok(0));
        assert_eq!(plan.field_offset(1, 0), Ok(8));
        assert_eq!(plan.field_offset(2, 0), Ok(16));
        assert_eq!(plan.field_offset(1, 2), Ok(56));
        assert_eq!(plan.field_offset(2, 2), Ok(64));
    }

    #[test]
    fn soa_layout_inverts_field_storage_without_misalignment() {
        let plan = plan_layout(&SHAPE, 3, LayoutMode::Soa, AddressWidth::Bits32, 40).unwrap();
        assert_eq!(plan.record_stride, None);
        assert_eq!(plan.allocation_bytes, 40);
        assert_eq!(plan.field_offset(0, 2), Ok(2));
        assert_eq!(plan.field_offset(1, 0), Ok(8));
        assert_eq!(plan.field_offset(1, 2), Ok(24));
        assert_eq!(plan.field_offset(2, 0), Ok(32));
        assert_eq!(plan.field_offset(2, 2), Ok(36));
        for placement in &plan.fields {
            assert_eq!(placement.offset % placement.alignment, 0);
        }
    }

    #[test]
    fn zero_capacity_is_safe_but_yields_no_valid_elements() {
        for mode in [LayoutMode::Aos, LayoutMode::Soa] {
            let plan = plan_layout(&SHAPE, 0, mode, AddressWidth::Bits32, 0).unwrap();
            assert_eq!(plan.allocation_bytes, 0);
            assert_eq!(plan.field_offset(0, 0), Err(LayoutError::UnknownElement));
        }
    }

    #[test]
    fn reject_untrusted_shapes_and_indices() {
        assert_eq!(
            plan_layout(&[], 1, LayoutMode::Aos, AddressWidth::Bits64, 1024),
            Err(LayoutError::EmptyShape)
        );
        let too_many = vec![ScalarField::U8; MAX_FIELDS + 1];
        assert_eq!(
            plan_layout(&too_many, 1, LayoutMode::Soa, AddressWidth::Bits64, 2048),
            Err(LayoutError::TooManyFields)
        );
        let plan = plan_layout(&SHAPE, 2, LayoutMode::Aos, AddressWidth::Bits32, 48).unwrap();
        assert_eq!(plan.field_offset(3, 0), Err(LayoutError::UnknownField));
        assert_eq!(plan.field_offset(0, 2), Err(LayoutError::UnknownElement));
        assert_eq!(plan.field_offset(0, u64::MAX), Err(LayoutError::UnknownElement));
    }

    #[test]
    fn checked_arithmetic_and_target_quotas_fail_closed() {
        for mode in [LayoutMode::Aos, LayoutMode::Soa] {
            assert_eq!(
                plan_layout(&[ScalarField::U64], u64::MAX, mode, AddressWidth::Bits64, u64::MAX),
                Err(LayoutError::ArithmeticOverflow)
            );
            assert_eq!(
                plan_layout(&SHAPE, 3, mode, AddressWidth::Bits64, 1),
                Err(LayoutError::BudgetExceeded)
            );
            assert_eq!(
                plan_layout(
                    &[ScalarField::U8],
                    (u32::MAX as u64) + 1,
                    mode,
                    AddressWidth::Bits32,
                    u64::MAX
                ),
                Err(LayoutError::BudgetExceeded)
            );
        }
    }

    #[test]
    fn all_scalar_shapes_have_nonoverlapping_in_bounds_fields() {
        let shapes: &[&[ScalarField]] = &[
            &[ScalarField::U8],
            &[ScalarField::U16, ScalarField::U8],
            &[ScalarField::U8, ScalarField::I64, ScalarField::F32],
            &[
                ScalarField::U8,
                ScalarField::U16,
                ScalarField::U32,
                ScalarField::U64,
                ScalarField::F32,
                ScalarField::F64,
            ],
        ];
        for shape in shapes {
            for capacity in 1..=16 {
                for mode in [LayoutMode::Aos, LayoutMode::Soa] {
                    let plan =
                        plan_layout(shape, capacity, mode, AddressWidth::Bits32, 4096).unwrap();
                    let mut covered = vec![false; plan.allocation_bytes() as usize];
                    for (index, placement) in plan.fields().iter().enumerate() {
                        for element in 0..plan.capacity() {
                            let offset = plan.field_offset(index, element).unwrap();
                            assert_eq!(offset % placement.alignment, 0);
                            for byte in offset..offset + placement.bytes {
                                let location = byte as usize;
                                assert!(location < covered.len());
                                assert!(!covered[location], "overlapping {mode:?} field bytes");
                                covered[location] = true;
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn declared_layouts_cannot_be_overridden_by_trace_hints() {
        let field_heavy = AccessCounts {
            object_major: 1,
            field_major: 100,
        };
        let object_heavy = AccessCounts {
            object_major: 100,
            field_major: 1,
        };
        for counts in [field_heavy, object_heavy] {
            assert_eq!(LayoutIntent::OrdinaryArray.resolve(counts), LayoutMode::Aos);
            assert_eq!(LayoutIntent::InvertedArray.resolve(counts), LayoutMode::Soa);
            assert_eq!(LayoutIntent::ExplicitAos.resolve(counts), LayoutMode::Aos);
            assert_eq!(LayoutIntent::ExplicitSoa.resolve(counts), LayoutMode::Soa);
        }
        assert_eq!(LayoutIntent::Auto.resolve(field_heavy), LayoutMode::Soa);
        assert_eq!(LayoutIntent::Auto.resolve(object_heavy), LayoutMode::Aos);
    }

    #[test]
    fn canonical_boolean_byte_is_one_wide_and_one_aligned() {
        assert_eq!(ScalarField::Bool.bytes(), 1);
        assert_eq!(ScalarField::Bool.alignment(), 1);
        for mode in [LayoutMode::Aos, LayoutMode::Soa] {
            let plan = plan_layout(
                &[ScalarField::I64, ScalarField::Bool, ScalarField::F64],
                3,
                mode,
                AddressWidth::Bits32,
                80,
            )
            .unwrap();
            assert_eq!(plan.fields()[1].kind, ScalarField::Bool);
            assert_eq!(plan.allocation_alignment(), 8);
            assert_eq!(plan.field_offset(0, 3), Err(LayoutError::UnknownElement));
        }
    }

    #[test]
    fn aot_layout_selection_is_static_not_a_live_rewrite() {
        assert_eq!(AccessCounts::default().suggested_layout(), LayoutMode::Aos);
        assert_eq!(
            AccessCounts {
                object_major: 9,
                field_major: 9,
            }
            .suggested_layout(),
            LayoutMode::Aos
        );
        assert_eq!(
            AccessCounts {
                object_major: 8,
                field_major: 100,
            }
            .suggested_layout(),
            LayoutMode::Soa
        );
    }
}
