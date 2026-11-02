//! 应用构建身份属于公共诊断，与媒体运行时独立。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildFingerprint {
    pub git_commit_short: String,
    pub source_fingerprint_sha256: Option<String>,
    pub workspace_dirty: bool,
    pub build_timestamp_unix_ms: String,
    pub cargo_profile: String,
    pub edition: String,
}

pub fn current_build_fingerprint() -> BuildFingerprint {
    BuildFingerprint {
        git_commit_short: env!("XBX_BUILD_GIT_COMMIT_SHORT").to_string(),
        source_fingerprint_sha256: Some(env!("XBX_BUILD_SOURCE_SHA256").to_string()),
        workspace_dirty: env!("XBX_BUILD_WORKSPACE_DIRTY") == "true",
        build_timestamp_unix_ms: env!("XBX_BUILD_TIMESTAMP_UNIX_MS").to_string(),
        cargo_profile: env!("XBX_BUILD_CARGO_PROFILE").to_string(),
        edition: env!("XBX_BUILD_EDITION").to_string(),
    }
}
