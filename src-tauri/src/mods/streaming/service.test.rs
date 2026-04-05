use super::{
    apply_streaming_preferences, build_console_ready_snapshot, build_fallback_progress_snapshot,
    build_smartglass_ready_candidates, build_startup_diagnostic_summary_fallback,
    classify_startup_error_kind_fallback, is_startup_error_retryable_fallback,
    map_domain_progress_snapshot, parse_audio_bitrate_preference, parse_bitrate_preference,
    parse_codec_preference, parse_runtime_preference, startup_error_message_key, SessionFlowError,
    StreamingStartupErrorKind, StreamingStartupPhase,
};
use crate::mods::data::DataHostSummary;
use crate::mods::streaming::types::{
    StreamingConfigSnapshot, StreamingDisplayOptionsValue, StreamingSessionPhase,
    StreamingSessionProgressSnapshot,
};
use serde_json::json;
use xbox_streaming::{
    BitratePreference, CodecPreference, Config as DomainStreamingConfig, RuntimePreference,
    SessionFlowStartupErrorHint as DomainStartupErrorHint,
    SessionFlowStartupErrorKind as DomainStartupErrorKind, SessionPhase as DomainSessionPhase,
    SessionProgressSnapshot as DomainSessionProgressSnapshot,
};

#[test]
fn maps_http_error_with_status_and_body() {
    let error = SessionFlowError::http(503, "HTTP 503: error", Some("body".to_string()));
    assert_eq!(error.status, Some(503));
    assert_eq!(error.body.as_deref(), Some("body"));
}

#[test]
fn apply_streaming_preferences_maps_runtime_and_negotiation_fields() {
    let mut config = DomainStreamingConfig::default();
    let snapshot = StreamingConfigSnapshot {
        resolution: 1080,
        xhome_resolution: 1080,
        preferred_game_language: "en-US".to_string(),
        ipv6: false,
        force_region_ip: String::new(),
        xhome_bitrate_mode: "Custom".to_string(),
        xhome_bitrate: 35,
        xcloud_bitrate_mode: "Custom".to_string(),
        xcloud_bitrate: 18,
        audio_bitrate_mode: "Custom".to_string(),
        audio_bitrate: 2,
        codec: "video/H264-42e".to_string(),
        polling_rate: 333,
        vibration: false,
        vibration_strength: "enhanced".to_string(),
        stream_runtime_mode: "rust-owned".to_string(),
        power_on: true,
        server_url: "turn:example.test:3478".to_string(),
        server_username: "user".to_string(),
        server_credential: "secret".to_string(),
        xhome_turn_fallback: true,
        enable_audio_control: true,
        video_format: "Zoom".to_string(),
        display_options: StreamingDisplayOptionsValue {
            sharpness: 5,
            saturation: 110,
            contrast: 90,
            brightness: 105,
        },
    };

    apply_streaming_preferences(&mut config, &snapshot);

    assert_eq!(config.negotiation.video_codec, CodecPreference::H264Normal);
    assert_eq!(
        config.negotiation.home_video_bitrate,
        BitratePreference::CustomKbps { kbps: 35_000 }
    );
    assert_eq!(
        config.negotiation.cloud_video_bitrate,
        BitratePreference::CustomKbps { kbps: 18_000 }
    );
    assert_eq!(
        config.negotiation.audio_bitrate,
        BitratePreference::CustomKbps { kbps: 2 }
    );
    assert_eq!(config.input.polling_rate_hz, 333);
    assert!(!config.input.vibration);
    assert!(config.session.power_on);
    assert_eq!(config.runtime.mode, RuntimePreference::RustOwned);
    assert_eq!(config.runtime.home_fallback_turn, true);
    assert!(config.render.enable_audio_control);
    assert_eq!(config.render.video_format.as_deref(), Some("Zoom"));
    assert_eq!(config.render.display_options.sharpness, 5);
    assert_eq!(
        config
            .runtime
            .custom_turn
            .as_ref()
            .map(|turn| turn.url.as_str()),
        Some("turn:example.test:3478")
    );
}

#[test]
fn home_remote_play_not_ready_maps_to_host_remote_play_unavailable() {
    let error = SessionFlowError::message(
            "homeRemotePlayNotReady:targetId=console-1;powerState=On;remoteManagementEnabled=null;consoleStreamingEnabled=null;consoleAddrsCount=0;attempts=3;elapsedMs=8000;hint=hostRemotePlayUnavailable",
        );

    let phase = StreamingStartupPhase::ResolvingContext;
    let kind = classify_startup_error_kind_fallback(&phase, &error);

    assert_eq!(kind, StreamingStartupErrorKind::HostRemotePlayUnavailable);
    assert_eq!(
        startup_error_message_key(&kind),
        "streamPage.errors.hostRemotePlayUnavailable"
    );
    assert!(!is_startup_error_retryable_fallback(&kind, &error));
    assert!(build_startup_diagnostic_summary_fallback(&phase, &error)
        .contains("hint=hostRemotePlayUnavailable"));
}

#[test]
fn host_registration_retry_exhausted_maps_to_host_issue() {
    let error = SessionFlowError {
            message: "homeSessionBoundedRetryExhausted:targetId=console-1;reason=waitingForServerRegistration;retryCount=1;retryLimit=1".to_string(),
            status: None,
            body: Some(
                "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister"
                    .to_string(),
            ),
            startup_hint: None,
        };

    let phase = StreamingStartupPhase::WaitingSessionReady;
    let kind = classify_startup_error_kind_fallback(&phase, &error);

    assert_eq!(
        kind,
        StreamingStartupErrorKind::HostRegistrationRetryExhausted
    );
    assert_eq!(
        startup_error_message_key(&kind),
        "streamPage.errors.hostRegistrationRetryExhausted"
    );
    assert!(!is_startup_error_retryable_fallback(&kind, &error));
    assert!(build_startup_diagnostic_summary_fallback(&phase, &error)
        .contains("hint=hostRegistrationRetryExhausted"));
}

#[test]
fn domain_progress_hint_maps_to_structured_progress_error() {
    let progress = map_domain_progress_snapshot(DomainSessionProgressSnapshot {
            session_id: "session-1".to_string(),
            phase: DomainSessionPhase::Failed,
            status_text_key: "streamPage.errors.startFailed".to_string(),
            queue_seconds: None,
            queue: None,
            error_code: Some("ServerNeverRegistered".to_string()),
            error_message: Some(
                "homeSessionBoundedRetryExhausted:targetId=console-1;reason=waitingForServerRegistration;retryCount=1;retryLimit=1"
                    .to_string(),
            ),
            error_hint: Some(DomainStartupErrorHint {
                kind: DomainStartupErrorKind::HostRegistrationRetryExhausted,
                retryable: false,
                diagnostic_summary: "targetId=console-1; reason=waitingForServerRegistration; retryCount=1; retryLimit=1; hint=hostRegistrationRetryExhausted".to_string(),
            }),
        });

    assert_eq!(
        progress
            .error
            .as_ref()
            .map(|error| error.error_kind.clone()),
        Some(StreamingStartupErrorKind::HostRegistrationRetryExhausted)
    );
    assert_eq!(
        progress
            .error
            .as_ref()
            .and_then(|error| error.bounded_retry.as_ref())
            .map(|retry| retry.retry_count),
        Some(1)
    );
}

#[test]
fn fallback_progress_registration_message_maps_structured_error() {
    let progress = build_fallback_progress_snapshot(StreamingSessionProgressSnapshot {
            session_id: "session-1".to_string(),
            phase: StreamingSessionPhase::Failed,
            status_text_key: "streamPage.errors.startFailed".to_string(),
            queue_seconds: None,
            queue: None,
            error_code: Some("ServerNeverRegistered".to_string()),
            error_message: Some(
                "homeSessionBoundedRetryExhausted:targetId=console-1;reason=waitingForServerRegistration;retryCount=1;retryLimit=1"
                    .to_string(),
            ),
            error: None,
        });

    assert_eq!(
        progress
            .error
            .as_ref()
            .map(|error| error.error_kind.clone()),
        Some(StreamingStartupErrorKind::HostRegistrationRetryExhausted)
    );
    assert_eq!(
        progress
            .error
            .as_ref()
            .and_then(|error| error.bounded_retry.as_ref())
            .map(|retry| retry.retry_limit),
        Some(1)
    );
    assert_eq!(
        progress.error.as_ref().map(|error| error.retryable),
        Some(false)
    );
}

#[test]
fn fallback_progress_without_raw_error_keeps_structured_error_empty() {
    let progress = build_fallback_progress_snapshot(StreamingSessionProgressSnapshot {
        session_id: "session-1".to_string(),
        phase: StreamingSessionPhase::WaitingSessionReady,
        status_text_key: "streamPage.status.waitingSession".to_string(),
        queue_seconds: None,
        queue: None,
        error_code: None,
        error_message: None,
        error: None,
    });

    assert!(progress.error.is_none());
}

#[test]
fn fallback_progress_network_message_maps_retryable_network_error() {
    let progress = build_fallback_progress_snapshot(StreamingSessionProgressSnapshot {
        session_id: "session-1".to_string(),
        phase: StreamingSessionPhase::Recovering,
        status_text_key: "streamPage.status.reconnecting".to_string(),
        queue_seconds: None,
        queue: None,
        error_code: None,
        error_message: Some("networkLost reconnecting".to_string()),
        error: None,
    });

    assert_eq!(
        progress
            .error
            .as_ref()
            .map(|error| error.error_kind.clone()),
        Some(StreamingStartupErrorKind::Network)
    );
    assert_eq!(
        progress.error.as_ref().map(|error| error.retryable),
        Some(true)
    );
}

#[test]
fn build_console_ready_snapshot_includes_smartglass_ready_hosts() {
    let smartglass = vec![DataHostSummary {
        id: Some("console-1".to_string()),
        power_state: Some("On".to_string()),
        remote_management_enabled: Some(true),
        console_streaming_enabled: Some(true),
        ..Default::default()
    }];
    let smartglass_ready = build_smartglass_ready_candidates(&smartglass);

    let snapshot = build_console_ready_snapshot(&smartglass, &smartglass_ready);

    assert_eq!(snapshot["smartglassCount"], json!(1));
    assert_eq!(snapshot["smartglassReadyCount"], json!(1));
    assert_eq!(
        snapshot["smartglassHosts"][0]["remoteManagementEnabled"],
        json!(true)
    );
    assert_eq!(
        snapshot["smartglassReadyConsoles"][0]["readySource"],
        json!("smartglass")
    );
}

#[test]
fn build_smartglass_ready_candidates_keeps_smartglass_only_host_ready() {
    let smartglass = vec![DataHostSummary {
        id: Some("console-2".to_string()),
        power_state: Some("On".to_string()),
        remote_management_enabled: Some(true),
        console_streaming_enabled: Some(true),
        ..Default::default()
    }];

    let smartglass_ready = build_smartglass_ready_candidates(&smartglass);

    assert_eq!(smartglass_ready.len(), 1);
    assert_eq!(smartglass_ready[0].id.as_deref(), Some("console-2"));
    assert_eq!(smartglass_ready[0].server_id.as_deref(), Some("console-2"));
    assert_eq!(smartglass_ready[0].power_state.as_deref(), Some("On"));
    assert_eq!(smartglass_ready[0].remote_management_enabled, Some(true));
    assert_eq!(
        smartglass_ready[0].ready_source.as_deref(),
        Some("smartglass")
    );
}

#[test]
fn parse_helpers_fall_back_to_auto_when_values_are_empty() {
    assert_eq!(
        parse_bitrate_preference("Auto", 20),
        BitratePreference::Auto
    );
    assert_eq!(
        parse_audio_bitrate_preference("Auto", 24),
        BitratePreference::Auto
    );
    assert_eq!(
        parse_audio_bitrate_preference("Custom", 24),
        BitratePreference::CustomKbps { kbps: 24 }
    );
    assert_eq!(parse_codec_preference(""), CodecPreference::Auto);
    assert_eq!(
        parse_codec_preference("video/H264-64"),
        CodecPreference::H264High
    );
    assert_eq!(
        parse_codec_preference("video/H264-4d"),
        CodecPreference::H264Main
    );
    assert_eq!(
        parse_codec_preference("video/H264-42e"),
        CodecPreference::H264Normal
    );
    assert_eq!(
        parse_codec_preference("video/H264-420"),
        CodecPreference::H264Low
    );
    assert_eq!(parse_runtime_preference(""), RuntimePreference::Auto);
}
