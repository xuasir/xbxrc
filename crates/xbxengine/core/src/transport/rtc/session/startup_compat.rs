use std::sync::Mutex;

use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::video_scheduling_owner::RecoveryIntentSource;
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::contract::idr_recovery_active_from_stats;
use crate::XbxEngineMediaRuntimeStats;

fn recovery_surface_blocks_startup_compat(
    stats: &XbxEngineMediaRuntimeStats,
    observed_at_ms: f64,
) -> bool {
    idr_recovery_active_from_stats(stats, observed_at_ms)
}

pub(crate) fn should_absorb_first_frame_acquisition_anchor_issue(
    snapshot: &TransportSnapshot,
    chain_reason: Option<&str>,
    source_event: &str,
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    pre_first_frame_reconnect_fallback_ms: f64,
) -> bool {
    if source_event != "frame-inspection-rejected-await-anchor"
        || snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
    {
        return false;
    }
    if !chain_reason.is_some_and(is_first_frame_acquisition_reason_label) {
        return false;
    }
    first_frame_acquisition_window_active(
        runtime_stats,
        snapshot.now_ms,
        pre_first_frame_reconnect_fallback_ms,
        true,
    )
}

pub(crate) fn first_frame_acquisition_priority_active(
    snapshot: &TransportSnapshot,
    observed_at_ms: f64,
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    pre_first_frame_reconnect_fallback_ms: f64,
) -> bool {
    if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected {
        return false;
    }
    first_frame_acquisition_window_active(
        runtime_stats,
        observed_at_ms,
        pre_first_frame_reconnect_fallback_ms,
        true,
    )
}

pub(crate) fn should_hold_pre_first_frame_connected_idle_timeout(
    snapshot: &TransportSnapshot,
    diagnosis_label: &str,
    observed_at_ms: f64,
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    pre_first_frame_reconnect_fallback_ms: f64,
) -> bool {
    if diagnosis_label != "adapterIdleTimeout" {
        return false;
    }
    if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
        || snapshot.media.frame_count != 0
    {
        return false;
    }
    first_frame_acquisition_window_active(
        runtime_stats,
        observed_at_ms,
        pre_first_frame_reconnect_fallback_ms,
        false,
    )
}

pub(crate) fn should_hold_pre_first_frame_display_supply_degraded(
    snapshot: &TransportSnapshot,
    source: RecoveryIntentSource,
    reason_label: &str,
    observed_at_ms: f64,
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    pre_first_frame_reconnect_fallback_ms: f64,
) -> bool {
    if source != RecoveryIntentSource::Supply
        || reason_label != "displaySupplyDegraded"
        || snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
        || snapshot.media.frame_count != 0
    {
        return false;
    }
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        if stats.transport_state != XbxEngineTransportStateDto::Connected {
            return false;
        }
        if recovery_surface_blocks_startup_compat(stats, observed_at_ms) {
            return false;
        }
        let Some(track) = stats.latest_video_track_status.as_ref() else {
            return false;
        };
        if track.state != "remoteTrackAttached" || track.video_bytes_total == 0 {
            return false;
        }
        if !pre_first_frame_fallback_within_window(
            stats,
            observed_at_ms,
            pre_first_frame_reconnect_fallback_ms,
        ) {
            return false;
        }
        stats.latest_video_host_present_time_ms.is_none()
            && stats.latest_video_decode_ok_time_ms.is_none()
    })
    .unwrap_or(false)
}

fn first_frame_acquisition_window_active(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    observed_at_ms: f64,
    pre_first_frame_reconnect_fallback_ms: f64,
    _require_startup_surface: bool,
) -> bool {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        if stats.transport_state != XbxEngineTransportStateDto::Connected {
            return false;
        }
        if recovery_surface_blocks_startup_compat(stats, observed_at_ms) {
            return false;
        }
        let Some(track) = stats.latest_video_track_status.as_ref() else {
            return false;
        };
        if track.state != "remoteTrackAttached" || track.video_bytes_total == 0 {
            return false;
        }
        if !pre_first_frame_fallback_within_window(
            stats,
            observed_at_ms,
            pre_first_frame_reconnect_fallback_ms,
        ) {
            return false;
        }
        stats.latest_video_host_present_time_ms.is_none()
            && stats.latest_video_decode_ok_time_ms.is_none()
    })
    .unwrap_or(false)
}

/// 首帧前抑制/保护：以首路视频包到达时间为锚，超过 `pre_first_frame_reconnect_fallback_ms` 即失效。
/// 无 `first_video_packet_arrival_time_ms` 时：仅在显式 `session_phase=priming` 且已附着视频轨时，
/// 用 `latest_video_track_status.observed_at_ms` 作为临时窗口锚（与 `VideoSchedulingOwnerInput`
/// 单测里 `first_frame_acquisition_priority_allowed: true` 的语义对齐），仍受同一毫秒上限约束，
/// 避免无首包时间戳时首帧采集优先门控永久失效。
fn pre_first_frame_fallback_within_window(
    stats: &XbxEngineMediaRuntimeStats,
    observed_at_ms: f64,
    pre_first_frame_reconnect_fallback_ms: f64,
) -> bool {
    match stats.first_video_packet_arrival_time_ms {
        Some(t0) => (observed_at_ms - t0).max(0.0) <= pre_first_frame_reconnect_fallback_ms,
        None => {
            if stats.session_phase.as_deref() != Some("priming") {
                return false;
            }
            let Some(track) = stats.latest_video_track_status.as_ref() else {
                return false;
            };
            if track.state != "remoteTrackAttached" || track.video_bytes_total == 0 {
                return false;
            }
            (observed_at_ms - track.observed_at_ms).max(0.0)
                <= pre_first_frame_reconnect_fallback_ms
        }
    }
}

fn is_first_frame_acquisition_reason_label(value: &str) -> bool {
    matches!(
        value,
        "bootstrapMissingSps"
            | "bootstrapMissingPps"
            | "inspectionRejectInvalidSliceHeader"
            | "bootstrapMissingIdr"
            | "mixedIdrWithTrailingDelta"
            | "receiverWaitingKeyframe"
            | "ingressWaitKeyframe"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
    use crate::transport::rtc::projection::{
        BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
        RecoveryProjection, TransportSnapshot,
    };
    use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoTrackStatus};

    fn connected_snapshot(now_ms: f64) -> TransportSnapshot {
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        TransportSnapshot::new(
            1,
            now_ms,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        )
    }

    fn pre_first_frame_stats(now_ms: f64) -> XbxEngineMediaRuntimeStats {
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            session_phase: Some("priming".to_string()),
            first_video_packet_arrival_time_ms: Some((now_ms - 50.0).max(0.0)),
            latest_video_track_status: Some(XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/h264".to_string()),
                transport_state: XbxEngineTransportStateDto::Connected,
                video_bytes_total: 128_000,
                video_packet_count_total: 96,
                audio_bytes_total: 16_000,
                observed_at_ms: now_ms,
            }),
            latest_video_host_present_time_ms: None,
            latest_video_decode_ok_time_ms: None,
            ..Default::default()
        }
    }

    #[test]
    fn first_frame_probe_keeps_startup_priority_window_active() {
        let now_ms = 100.0;
        let snapshot = connected_snapshot(now_ms);
        let mut stats = pre_first_frame_stats(now_ms);
        stats.latest_keyframe_request_source = Some("first-frame-acquisition".to_string());
        stats.latest_keyframe_request_outcome = Some("sent".to_string());
        stats.receive_keyframe_last_sent_at_ms = Some(now_ms - 5.0);
        stats.receive_keyframe_required = Some(false);
        stats.receive_keyframe_required_cause = Some("none".to_string());
        let runtime_stats = Mutex::new(stats);

        assert!(first_frame_acquisition_priority_active(
            &snapshot,
            now_ms,
            &runtime_stats,
            1_500.0,
        ));
    }

    #[test]
    fn blocking_receive_recovery_closes_startup_priority_window() {
        let now_ms = 100.0;
        let snapshot = connected_snapshot(now_ms);
        let mut stats = pre_first_frame_stats(now_ms);
        stats.receive_keyframe_required = Some(true);
        stats.receive_keyframe_required_cause = Some("decoder-waiting-keyframe".to_string());
        let runtime_stats = Mutex::new(stats);

        assert!(!first_frame_acquisition_priority_active(
            &snapshot,
            now_ms,
            &runtime_stats,
            1_500.0,
        ));
    }
}
