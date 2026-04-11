//! 播放期集成测试共享：墙钟对齐、TWCC 片段、禁止 reconnect 等。

use crate::api::backend::{
    XbxEngineMediaRuntimeStats, XbxEngineTwccObservationQuality, XbxEngineVideoTwccObservation,
};
use crate::transport::rtc::facts::TransportCommand;

pub(crate) fn wall_observed_ms() -> f64 {
    crate::transport::rtc::stats::now_ms_f64()
}

pub(crate) fn assert_cmds_have_no_reconnect(cmds: &[TransportCommand], case_id: &str) {
    assert!(
        cmds.iter()
            .all(|c| !matches!(c, TransportCommand::RequestReconnectCandidate { .. })),
        "{case_id}: 禁止无证据 reconnect candidate，cmds={cmds:?}"
    );
}

pub(crate) fn fill_twcc_stable_local_feedback(stats: &mut XbxEngineMediaRuntimeStats, obs_wall: f64) {
    stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
        observation_id: 700,
        source: "local-feedback".to_string(),
        feedback_packet_count: 40,
        covered_sequence_start: 1,
        covered_sequence_end: 120,
        covered_sequence_span: 120,
        observed_packet_count: 40,
        observed_byte_count: 24_000,
        coverage_ratio: Some(0.98),
        ledger_hit_ratio: Some(0.96),
        feedback_interval_ms: Some(50.0),
        arrival_span_ms: Some(48.0),
        receive_bitrate_kbps: Some(8_000.0),
        twcc_sample_valid: true,
        twcc_invalid_reason: None,
        quality: XbxEngineTwccObservationQuality::Stable,
        delivery_ratio: 0.99,
        packet_loss_ratio: 0.002,
        observed_at_ms: obs_wall - 1.0,
    });
}
