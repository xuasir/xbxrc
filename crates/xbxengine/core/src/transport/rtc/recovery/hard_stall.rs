use std::sync::Mutex;

use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationDecision, VideoEscalationReason,
};
use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
use crate::XbxEngineMediaRuntimeStats;

pub(crate) const HARD_STALL_DECODER_RESET_MS: f64 = 1_200.0;
pub(crate) const HARD_STALL_SAMPLE_AGE_FALLBACK_MS: f64 = 3_000.0;
pub(crate) const HARD_STALL_MIN_RESET_SPACING_MS: f64 = 1_200.0;

struct HardStallSnapshot {
    transport_state: XbxEngineTransportStateDto,
    inbound_video_bitrate_kbps: f64,
    present_age_ms: f64,
    packet_age_ms: f64,
    effective_present_fps: f64,
    direct_gaming_bitrate_band: Option<String>,
    since_last_decoder_reset_ms: f64,
}

pub(crate) fn resolve_persistent_stall_recovery(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    reason: &VideoEscalationReason,
) -> Option<VideoEscalationDecision> {
    if !matches!(
        reason,
        VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSevereDeadline
    ) {
        return None;
    }

    let snapshot = read_hard_stall_snapshot(runtime_stats)?;
    if snapshot.transport_state != XbxEngineTransportStateDto::Connected {
        return None;
    }
    if !snapshot.is_hard_paused_stream() {
        return None;
    }

    // 连接域 deadline 不在媒体硬停顿旁路里发 decoder reset，避免把传输坏窗误当成本地解码器问题；
    // 升级由 escalation 的连接域路径（reconnect / cooldown）处理。
    if matches!(
        reason,
        VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSevereDeadline
    ) {
        return None;
    }

    if snapshot.since_last_decoder_reset_ms >= HARD_STALL_MIN_RESET_SPACING_MS {
        return Some(VideoEscalationDecision {
            observation_id: 0,
            action: RecoveryAction::RequestDecoderReset,
        });
    }

    None
}

fn read_hard_stall_snapshot(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> Option<HardStallSnapshot> {
    let now_ms = unix_now_ms();
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let present_age_ms = stats
            .latest_video_host_present_time_ms
            .map(|at_ms| (now_ms - at_ms).max(0.0))
            .unwrap_or(HARD_STALL_SAMPLE_AGE_FALLBACK_MS);
        let effective_present_fps = if stats.video_renderer_stalled.unwrap_or(false)
            || present_age_ms >= HARD_STALL_DECODER_RESET_MS
        {
            0.0
        } else {
            stats.video_present_fps
        };
        Some(HardStallSnapshot {
            transport_state: stats.transport_state.clone(),
            inbound_video_bitrate_kbps: stats.inbound_video_bitrate_kbps.unwrap_or(0.0),
            present_age_ms,
            packet_age_ms: stats
                .latest_video_packet_arrival_time_ms
                .map(|at_ms| (now_ms - at_ms).max(0.0))
                .unwrap_or(HARD_STALL_SAMPLE_AGE_FALLBACK_MS),
            effective_present_fps,
            direct_gaming_bitrate_band: stats.direct_gaming_bitrate_band.clone(),
            since_last_decoder_reset_ms: stats
                .latest_video_decoder_reset_time_ms
                .map(|at_ms| (now_ms - at_ms).max(0.0))
                .unwrap_or(f64::INFINITY),
        })
    })
    .flatten()
}

impl HardStallSnapshot {
    fn is_hard_paused_stream(&self) -> bool {
        self.inbound_video_bitrate_kbps <= 0.1
            && self.direct_gaming_bitrate_band.as_deref() == Some("paused")
            && self.effective_present_fps <= 1.0
            && self.present_age_ms >= HARD_STALL_DECODER_RESET_MS
            && self.packet_age_ms >= HARD_STALL_DECODER_RESET_MS
    }
}
