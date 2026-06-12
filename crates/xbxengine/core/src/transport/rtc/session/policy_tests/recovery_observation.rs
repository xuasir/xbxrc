use super::super::RecoveryObservationSnapshot;

#[test]
fn recovery_observation_snapshot_allows_transport_await_reconnect_when_local_self_healing_exhausted(
) {
    let snapshot = RecoveryObservationSnapshot {
        ingress_active: true,
        reassembly_active: true,
        decode_active: true,
        render_active: true,
        rtc_connectivity_connected: true,
        reconnect_in_flight: false,
        stable_serving: false,
        last_media_progress_at: Some(1_000.0),
        last_video_decode_ok_at: Some(1_000.0),
        last_keyframe_requested_at: Some(1_100.0),
        last_keyframe_decoded_at: None,
        local_decoder_reset_count_in_window: 1,
        keyframe_request_count_in_window: 1,
        remote_recovery_terminal_active: false,
    };

    assert!(snapshot.allows_transport_await_reconnect_fallback(2_200.0));
}

#[test]
fn recovery_observation_snapshot_blocks_transport_await_reconnect_when_keyframe_window_not_exhausted(
) {
    let snapshot = RecoveryObservationSnapshot {
        ingress_active: true,
        reassembly_active: true,
        decode_active: true,
        render_active: true,
        rtc_connectivity_connected: true,
        reconnect_in_flight: false,
        stable_serving: false,
        last_media_progress_at: Some(1_000.0),
        last_video_decode_ok_at: Some(1_000.0),
        last_keyframe_requested_at: Some(1_950.0),
        last_keyframe_decoded_at: None,
        local_decoder_reset_count_in_window: 1,
        keyframe_request_count_in_window: 1,
        remote_recovery_terminal_active: false,
    };

    assert!(!snapshot.allows_transport_await_reconnect_fallback(2_200.0));
}

#[test]
fn recovery_observation_snapshot_allows_remote_terminal_inside_keyframe_window() {
    let snapshot = RecoveryObservationSnapshot {
        ingress_active: true,
        reassembly_active: true,
        decode_active: true,
        render_active: true,
        rtc_connectivity_connected: true,
        reconnect_in_flight: false,
        stable_serving: false,
        last_media_progress_at: Some(1_000.0),
        last_video_decode_ok_at: Some(1_000.0),
        last_keyframe_requested_at: Some(2_050.0),
        last_keyframe_decoded_at: None,
        local_decoder_reset_count_in_window: 0,
        keyframe_request_count_in_window: 0,
        remote_recovery_terminal_active: true,
    };

    assert!(snapshot.allows_transport_await_reconnect_fallback(2_200.0));
}

#[test]
fn recovery_observation_snapshot_ignores_stale_decode_and_render_presence() {
    let snapshot = RecoveryObservationSnapshot {
        ingress_active: false,
        reassembly_active: false,
        decode_active: false,
        render_active: false,
        rtc_connectivity_connected: true,
        reconnect_in_flight: false,
        stable_serving: false,
        last_media_progress_at: Some(1_000.0),
        last_video_decode_ok_at: Some(1_000.0),
        last_keyframe_requested_at: Some(1_100.0),
        last_keyframe_decoded_at: None,
        local_decoder_reset_count_in_window: 1,
        keyframe_request_count_in_window: 1,
        remote_recovery_terminal_active: false,
    };

    assert!(!snapshot.allows_transport_await_reconnect_fallback(2_200.0));
}
