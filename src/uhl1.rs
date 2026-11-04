//! UHL-1 native Cranelift capability negotiation, not an actor hot loader.
//! Cranelift currently emits a scalar JIT entry only: it cannot upgrade
//! actor mailboxes, supervisor child specs, ETS tables or continuation frames.
//! All feature bits stay zero until compiler-generated actor/ETS proof exists.

pub const ABI_VERSION: u32 = 1;
pub const FEATURE_BITS: u64 = 0;
pub const MAX_MANIFEST_BYTES: u64 = 0;
pub const MAX_CHECKPOINT_BYTES: u64 = 0;
pub const MAX_LIVE_GENERATIONS: u64 = 0;

/// No external JSON dependency: this is a fixed, audited ASCII result.
pub const CAPABILITIES_JSON: &str = concat!(
    "{\"interface\":\"ores.hotload/v1\",",
    "\"backend\":\"cranelift\",\"abiVersion\":1,",
    "\"features\":0,\"executionModes\":0,",
    "\"maxManifestBytes\":0,\"maxCheckpointBytes\":0,",
    "\"maxLiveGenerations\":0,",
    "\"guestControlsHotLoad\":false,",
    "\"productionActorHotLoad\":false}"
);

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Unsupported,
    BadAbi,
}

pub fn query(version: u32) -> Result<&'static str, Status> {
    if version != ABI_VERSION {
        return Err(Status::BadAbi);
    }
    Ok(CAPABILITIES_JSON)
}

pub fn stage(_source: &[u8]) -> Result<(), Status> {
    Err(Status::Unsupported)
}

pub fn commit(_transaction: u64, _expected_epoch: u64) -> Result<u64, Status> {
    Err(Status::Unsupported)
}

pub fn acknowledge_ets_heir(_receipt: u64, _heir: u64) -> Result<(), Status> {
    Err(Status::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_unsound_hotload_feature_flags() {
        assert_eq!(ABI_VERSION, 1);
        assert_eq!(FEATURE_BITS, 0);
        assert_eq!(MAX_MANIFEST_BYTES, 0);
        assert_eq!(MAX_CHECKPOINT_BYTES, 0);
        assert_eq!(MAX_LIVE_GENERATIONS, 0);
        assert_eq!(query(0), Err(Status::BadAbi));
        assert_eq!(query(2), Err(Status::BadAbi));
        assert_eq!(query(1), Ok(CAPABILITIES_JSON));
        assert!(CAPABILITIES_JSON.contains("\"guestControlsHotLoad\":false"));
    }

    #[test]
    fn forged_hotloads_and_ets_receipts_cannot_produce_jit_images() {
        let probe = [0_u8, 1, 2, 3, 4];
        for _ in 0..10_000 {
            assert_eq!(stage(&probe), Err(Status::Unsupported));
            assert_eq!(commit(7, 1), Err(Status::Unsupported));
            assert_eq!(acknowledge_ets_heir(11, 12), Err(Status::Unsupported));
        }
        assert_eq!(probe, [0, 1, 2, 3, 4]);
    }
}
