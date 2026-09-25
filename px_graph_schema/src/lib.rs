//! See docs/operators.md

pub mod cache;
pub mod contract;
pub mod identity;
pub mod keys;
pub mod ops;
pub mod payload;
pub mod protocol;

pub use cache::{Cache, Report};
pub use contract::{Cooked, PxInputs, PxOp, interface_hash};
pub use identity::blake3;
pub use identity::{FNV_OFFSET, FNV_PRIME, HashField, PxKeyed, fnv1a, fnv1a_bytes, fnv1a_sources};
pub use keys::{Key, OpId, canonical_params, hex, hex_short, node_key, payload_fingerprint};
pub mod build;

pub use build::Build;
pub use protocol::{GraphSpec, ManifestEntry};
#[doc(inline)]
pub use px_protocol::payload::PayloadBundle;

pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

pub const TOOLCHAIN_HASH: &str = env!("PX_TOOLCHAIN_HASH");
