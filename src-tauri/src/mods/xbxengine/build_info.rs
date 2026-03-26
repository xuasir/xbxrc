use xbxengine::XbxEngineRuntimeConfig;
use xbxengine_protocol::XbxEngineBuildFingerprintDto;

fn default_feedback_interval_ms() -> u64 {
    XbxEngineRuntimeConfig::default()
        .webrtc
        .video_pipeline
        .feedback_interval_ms
}

pub(crate) fn current_build_fingerprint() -> XbxEngineBuildFingerprintDto {
    current_build_fingerprint_with_effective(default_feedback_interval_ms())
}

pub(crate) fn current_build_fingerprint_with_effective(
    effective_feedback_interval_ms: u64,
) -> XbxEngineBuildFingerprintDto {
    XbxEngineBuildFingerprintDto {
        git_commit_short: env!("XBX_BUILD_GIT_COMMIT_SHORT").to_string(),
        workspace_dirty: env!("XBX_BUILD_WORKSPACE_DIRTY") == "true",
        build_timestamp_unix_ms: env!("XBX_BUILD_TIMESTAMP_UNIX_MS").to_string(),
        cargo_profile: env!("XBX_BUILD_CARGO_PROFILE").to_string(),
        default_feedback_interval_ms: default_feedback_interval_ms(),
        effective_feedback_interval_ms,
        controlled_twcc_registry: true,
    }
}
