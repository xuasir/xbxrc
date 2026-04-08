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
    _pre_first_frame_reconnect_fallback_ms: f64,
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
        let _ = observed_at_ms;
        stats.latest_video_host_present_time_ms.is_none()
            && stats.latest_video_decode_ok_time_ms.is_none()
    })
    .unwrap_or(false)
}

fn first_frame_acquisition_window_active(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    observed_at_ms: f64,
    _pre_first_frame_reconnect_fallback_ms: f64,
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
        let _ = observed_at_ms;
        stats.latest_video_host_present_time_ms.is_none()
            && stats.latest_video_decode_ok_time_ms.is_none()
    })
    .unwrap_or(false)
}

fn is_first_frame_acquisition_reason_label(value: &str) -> bool {
    matches!(
        value,
        "bootstrapMissingSps"
            | "bootstrapMissingPps"
            | "bootstrapInFlight"
            | "inspectionRejectInvalidSliceHeader"
            | "NonIdrVcl"
            | "transportAwaitRecoveryKeyframe"
            | "ingressWaitKeyframe"
    )
}
