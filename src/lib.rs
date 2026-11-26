//! Experimental trusted-host compiler API. The CLI has no cache authority.
//! Host callers must independently protect cache/authority directories,
//! compiler and toolchain identities, current pins and master keys.
//! No guest access, automatic registry trust or default CLI cache activation.

pub mod backend;
pub mod compiler_artifact_receipt;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod compiler_artifact_store;
pub mod compiler_cache_key;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod compiler_cache_key_policy;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod compiler_cache_ledger;
pub mod frontend;
pub mod isolate;
pub mod sha256;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use compiler_artifact_store::{
    authority::Authority,
    session::{Build, NativeSession},
};
