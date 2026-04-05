use super::{
    XbxEngineDecodeRenderSignal, XbxEngineMediaSignal, XbxEngineRecoveryAction,
    XbxEngineRecoveryRuntimeConfig, XbxEngineRecoveryRuntimeConfigOverride,
    XbxEngineRecoverySignals, XbxEngineRuntimeHealth, XbxEngineTransportSignal,
    DECODER_RESET_AFTER_KEYFRAME_WAIT_MS, KEYFRAME_REQUEST_STALL_MS, RECONNECT_STALL_MS,
};
use xbxengine_protocol::XbxEngineTransportStateDto;

#[test]
fn recovery_signals_request_keyframe_before_reconnect() {
    let health = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        ..Default::default()
    };
    let action = health.next_recovery_action_with_signals(
        1_000.0 + KEYFRAME_REQUEST_STALL_MS + 10.0,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(1_000.0),
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(1_000.0),
                latest_frame_presented_at_ms: Some(1_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal::default(),
        },
    );
    assert_eq!(action, Some(XbxEngineRecoveryAction::RequestVideoKeyframe));
}

#[test]
fn recovery_signals_request_reconnect_after_extended_stall() {
    let health = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        keyframe_requested_for_current_stall: true,
        ..Default::default()
    };
    let action = health.next_recovery_action_with_signals(
        1_000.0 + RECONNECT_STALL_MS + 10.0,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(1_000.0),
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(1_000.0),
                latest_frame_presented_at_ms: Some(1_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal::default(),
        },
    );
    assert!(matches!(
        action,
        Some(XbxEngineRecoveryAction::RequestReconnect(_))
    ));
}

#[test]
fn recovery_signals_hold_reconnect_when_audio_is_still_alive() {
    let health = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        keyframe_requested_for_current_stall: true,
        ..Default::default()
    };
    let action = health.next_recovery_action_with_signals(
        1_000.0 + RECONNECT_STALL_MS + 10.0,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(1_000.0),
                audio_stream_alive: true,
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(1_000.0),
                latest_frame_presented_at_ms: Some(1_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal::default(),
        },
    );
    assert_eq!(action, None);
}

#[test]
fn recovery_signals_hold_reconnect_when_twcc_feedback_is_still_alive() {
    let health = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        keyframe_requested_for_current_stall: true,
        decoder_reset_requested_for_current_stall: true,
        ..Default::default()
    };
    let action = health.next_recovery_action_with_signals(
        1_000.0 + RECONNECT_STALL_MS + 10.0,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(1_000.0),
                latest_twcc_feedback_at_ms: Some(1_000.0 + RECONNECT_STALL_MS + 5.0),
                audio_stream_alive: false,
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(1_000.0),
                latest_frame_presented_at_ms: Some(1_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal::default(),
        },
    );
    assert_eq!(action, None);
}

#[test]
fn recovery_signals_request_decoder_reset_after_keyframe_on_decode_stall() {
    let now_ms = 10_000.0;
    let request_keyframe = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        ..Default::default()
    }
    .next_recovery_action_with_signals(
        now_ms,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(now_ms - 50.0),
                audio_stream_alive: false,
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(now_ms - 3_000.0),
                latest_frame_presented_at_ms: Some(now_ms - 3_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal {
                decoder_stalled: Some(true),
                render_stalled: Some(false),
                allow_decoder_reset: true,
            },
        },
    );
    assert_eq!(
        request_keyframe,
        Some(XbxEngineRecoveryAction::RequestVideoKeyframe)
    );

    let request_decoder_reset = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        last_keyframe_request_at_ms: Some(now_ms - DECODER_RESET_AFTER_KEYFRAME_WAIT_MS - 10.0),
        keyframe_requested_for_current_stall: true,
        ..Default::default()
    }
    .next_recovery_action_with_signals(
        now_ms,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(now_ms - 50.0),
                audio_stream_alive: false,
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(now_ms - 3_000.0),
                latest_frame_presented_at_ms: Some(now_ms - 3_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal {
                decoder_stalled: Some(true),
                render_stalled: Some(false),
                allow_decoder_reset: true,
            },
        },
    );
    assert_eq!(
        request_decoder_reset,
        Some(XbxEngineRecoveryAction::RequestDecoderReset)
    );
}

#[test]
fn recovery_signals_request_keyframe_on_pipeline_stall_even_with_fresh_packets() {
    let now_ms = 10_000.0;
    let action = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        ..Default::default()
    }
    .next_recovery_action_with_signals(
        now_ms,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(now_ms - 20.0),
                latest_twcc_feedback_at_ms: Some(now_ms - 20.0),
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(now_ms - 5_000.0),
                latest_frame_presented_at_ms: Some(now_ms - 5_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal {
                decoder_stalled: Some(false),
                render_stalled: Some(true),
                allow_decoder_reset: true,
            },
        },
    );
    assert_eq!(action, Some(XbxEngineRecoveryAction::RequestVideoKeyframe));
}

#[test]
fn recovery_signals_request_reconnect_on_pipeline_stall_even_with_fresh_packets() {
    let now_ms = 10_000.0;
    let action = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        keyframe_requested_for_current_stall: true,
        decoder_reset_requested_for_current_stall: true,
        last_keyframe_request_at_ms: Some(now_ms - 4_000.0),
        last_decoder_reset_request_at_ms: Some(now_ms - 3_000.0),
        ..Default::default()
    }
    .next_recovery_action_with_signals(
        now_ms,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(now_ms - 20.0),
                latest_twcc_feedback_at_ms: Some(now_ms - 20.0),
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(now_ms - 9_000.0),
                latest_frame_presented_at_ms: Some(now_ms - 9_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal {
                decoder_stalled: Some(false),
                render_stalled: Some(true),
                allow_decoder_reset: true,
            },
        },
    );
    assert!(matches!(
        action,
        Some(XbxEngineRecoveryAction::RequestReconnect(_))
    ));
}

#[test]
fn recovery_config_override_changes_keyframe_trigger_threshold() {
    let health = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        ..Default::default()
    };
    let recovery_config = XbxEngineRecoveryRuntimeConfig {
        keyframe_request_stall_ms: 900,
        ..Default::default()
    };
    let action = health.next_recovery_action_with_signals_and_config(
        1_950.0,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(1_000.0),
                audio_stream_alive: false,
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(1_000.0),
                latest_frame_presented_at_ms: Some(1_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal::default(),
        },
        &recovery_config,
    );
    assert_eq!(action, Some(XbxEngineRecoveryAction::RequestVideoKeyframe));
}

#[test]
fn recovery_requests_decoder_reset_earlier_for_audio_alive_video_only_stall() {
    let health = XbxEngineRuntimeHealth {
        connected_at_ms: Some(1_000.0),
        keyframe_requested_for_current_stall: true,
        last_keyframe_request_at_ms: Some(1_900.0),
        ..Default::default()
    };
    let action = health.next_recovery_action_with_signals(
        2_250.0,
        true,
        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: true,
                connected_at_ms: Some(1_000.0),
                latest_video_packet_arrival_at_ms: Some(1_000.0),
                audio_stream_alive: true,
                ..Default::default()
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: Some(1_000.0),
                latest_frame_presented_at_ms: Some(1_000.0),
            },
            decode_render: XbxEngineDecodeRenderSignal {
                decoder_stalled: Some(false),
                render_stalled: Some(false),
                allow_decoder_reset: true,
            },
        },
    );
    assert_eq!(action, Some(XbxEngineRecoveryAction::RequestDecoderReset));
}

#[test]
fn recovery_override_applies_partial_fields_only() {
    let base = XbxEngineRecoveryRuntimeConfig {
        first_frame_grace_ms: 6_000,
        keyframe_request_stall_ms: 1_000,
        keyframe_loss_burst_threshold: 3,
        decoder_reset_after_keyframe_wait_ms: 350,
        decoder_reset_request_cooldown_ms: 1_000,
        reconnect_stall_ms: 2_800,
        stall_recovery_cooldown_ms: 4_000,
    };
    let override_config = XbxEngineRecoveryRuntimeConfigOverride {
        reconnect_stall_ms: Some(5_000),
        ..Default::default()
    };
    let merged = base.with_override(override_config);
    assert_eq!(merged.reconnect_stall_ms, 5_000);
    assert_eq!(
        merged.keyframe_request_stall_ms,
        base.keyframe_request_stall_ms
    );
    assert_eq!(
        merged.keyframe_loss_burst_threshold,
        base.keyframe_loss_burst_threshold
    );
}

#[test]
fn reset_video_epoch_clears_frame_tracking_without_touching_transport_state() {
    let mut health = XbxEngineRuntimeHealth {
        observed_transport_state: XbxEngineTransportStateDto::Connected,
        connected_at_ms: Some(1_000.0),
        last_frame_seq: 223,
        last_frame_rendered_at_ms: Some(2_000.0),
        inbound_video_packet_count_total: 88,
        last_video_packet_arrival_at_ms: Some(2_100.0),
        last_keyframe_request_at_ms: Some(1_800.0),
        last_decoder_reset_request_at_ms: Some(1_900.0),
        stall_candidate_started_at_ms: Some(1_700.0),
        keyframe_requested_for_current_stall: true,
        decoder_reset_requested_for_current_stall: true,
        ..Default::default()
    };

    health.reset_video_epoch();

    assert_eq!(
        health.observed_transport_state,
        XbxEngineTransportStateDto::Connected
    );
    assert_eq!(health.connected_at_ms, Some(1_000.0));
    assert_eq!(health.last_frame_seq, 0);
    assert_eq!(health.last_frame_rendered_at_ms, None);
    assert_eq!(health.inbound_video_packet_count_total, 0);
    assert_eq!(health.last_video_packet_arrival_at_ms, None);
    assert_eq!(health.last_keyframe_request_at_ms, None);
    assert_eq!(health.last_decoder_reset_request_at_ms, None);
    assert_eq!(health.stall_candidate_started_at_ms, None);
    assert!(!health.keyframe_requested_for_current_stall);
    assert!(!health.decoder_reset_requested_for_current_stall);
}
