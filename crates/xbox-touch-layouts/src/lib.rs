//! 官方 TAKX 的有界解包、默认状态引用和本地布局快照；输入与宿主存储沿各自边界装配。
mod archive;
mod bundle;
mod catalog;
mod community;
mod protocol;
mod references;
mod runtime;

pub use bundle::{decode_bundle, ResolvedTouchLayout, TouchAssetReference, TouchBundle};
pub use catalog::{title_bundles, TouchBundleDescriptor};
pub use community::{community_bundle, validate_community_definition};
pub use protocol::{
    compile_layout, compile_layout_json, protocol_information_json, protocol_revision,
    CompiledTouchLayout, TouchCompiledControl, TouchProtocolIssue, MAX_COMPILED_BYTES,
};
pub use runtime::resolve_runtime_document;

pub const MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_FILES: usize = 1024;
pub const MAX_LAYOUTS: usize = 256;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TouchBundleError(pub String);

pub(crate) fn invalid(reason: &str) -> TouchBundleError {
    TouchBundleError(reason.into())
}

pub fn content_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

#[cfg(test)]
#[path = "bundle.test.rs"]
mod tests;
