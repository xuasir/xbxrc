use std::sync::Mutex;

use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::video_scheduling_owner::RecoveryIntentSource;
use crate::transport::rtc::projection::TransportSnapshot;
use crate::XbxEngineMediaRuntimeStats;

pub(crate) fn should_absorb_first_frame_acquisition_anchor_issue(
    snapshot: &TransportSnapshot,
    chain_reason: Option<&str>,
    source_event: &str,
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    pre_first_frame_reconnect_fallback_ms: f64,
) -> bool {
    if source_event != "frame-inspection-rejected-await-keyframe"
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
            (observed_at_ms - track.observed_at_ms).max(0.0) <= pre_first_frame_reconnect_fallback_ms
        }
    }
}

fn is_first_frame_acquisition_reason_label(value: &str) -> bool {
    matches!(
        value,
        "bootstrapMissingSps"
            | "bootstrapMissingPps"
            | "recoverySustaining"
            | "inspectionRejectInvalidSliceHeader"
            | "NonIdrVcl"
            | "transportAwaitRecoveryKeyframe"
            | "ingressWaitKeyframe"
    )
}
