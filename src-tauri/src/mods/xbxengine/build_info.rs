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
    let common = crate::build_info::current_build_fingerprint();
    XbxEngineBuildFingerprintDto {
        git_commit_short: common.git_commit_short,
        source_fingerprint_sha256: common.source_fingerprint_sha256,
        workspace_dirty: common.workspace_dirty,
        build_timestamp_unix_ms: common.build_timestamp_unix_ms,
        cargo_profile: common.cargo_profile,
        default_feedback_interval_ms: default_feedback_interval_ms(),
        effective_feedback_interval_ms,
        controlled_twcc_registry: true,
    }
}
