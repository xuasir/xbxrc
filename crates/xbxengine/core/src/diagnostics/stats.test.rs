use xbxengine_protocol::XbxEngineTransportStateDto;

use super::*;
use crate::api::runtime::XbxEngineRuntimeSnapshot;

fn test_snapshot() -> XbxEngineRuntimeSnapshot {
    XbxEngineRuntimeSnapshot {
        audio_volume: 1.0,
        keyboard_pointer_enabled: false,
        microphone_capturing: false,
        microphone_paused: false,
        display_state: None,
        viewport: None,
        surface_id: None,
        video_size: None,
        last_keyboard_pointer_event: None,
        last_pressed_controller_button: None,
        negotiation_attempt_count: 0,
        last_offer_sdp: None,
        last_answer_sdp: None,
        last_remote_candidates: Vec::new(),
        input_device_count: 0,
        input_pad_count: 0,
        input_route_attached: false,
        first_frame_packet_arrival_time_ms: None,
        frame_decoded_time_ms: None,
        frame_rendered_time_ms: None,
        latest_video_track_status: None,
        recovery_keyframe_request_count: 0,
        recovery_decoder_reset_count: 0,
        recovery_reconnect_count: 0,
        last_recovery_action: None,
        last_recovery_action_at_ms: None,
        last_recovery_reason: None,
        reconnect_trigger_source: None,
    }
}

#[test]
fn audio_playout_latency_helpers_return_latency_and_av_delta() {
    let stats = XbxEngineMediaRuntimeStats {
        audio_playout_latency_ms: Some(42.5),
        ..XbxEngineMediaRuntimeStats::default()
    };

    assert_eq!(resolve_audio_playout_latency_ms(&stats), Some(42.5));
    assert_eq!(
        resolve_audio_video_playout_delta_ms(&stats, Some(30.0)),
        Some(12.5)
    );
    assert_eq!(resolve_audio_video_playout_delta_ms(&stats, None), None);
}

#[test]
fn runtime_summary_includes_transport_recovery_epoch_note() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("recovering".to_string()),
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(20.0),
        latest_video_host_present_time_ms: Some(30.0),
        video_present_submit_count_total: 1,
        direct_gaming_bitrate_band: Some("steady".to_string()),
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
        transport_recovery_epoch: 7,
        transport_recovery_epoch_at_last_escalation: 6,
        transport_recovery_episode_active: true,
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    let runtime_summary = dto.runtime_summary.expect("runtime summary");

    assert!(runtime_summary.contains("repoch:7:active"));
}

#[test]
fn runtime_summary_uses_remote_profile_input_and_owner_state_as_main_view() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        baseline_remote_profile: Some("cloudGaming".to_string()),
        dynamic_remote_subprofile: Some("cloudHighRtt".to_string()),
        effective_remote_profile_label: Some("cloudGaming+cloudHighRtt".to_string()),
        session_phase: Some("recovering".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    let summary = dto.runtime_summary.expect("runtime summary");
    assert!(summary
        .starts_with("cloudGaming+cloudHighRtt/handshaking/steady/rebuilding-supply/recovering"));
}

#[test]
fn stream_lifecycle_phase_projects_unified_semantics() {
    let startup = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connecting,
        ..XbxEngineMediaRuntimeStats::default()
    };
    let steady = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(10.0),
        latest_video_host_present_time_ms: Some(10.0),
        video_owner_state: Some("stable-serving".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };
    let degraded = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(10.0),
        latest_video_host_present_time_ms: Some(10.0),
        video_owner_state: Some("degraded-serving".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };
    let failed = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Failed,
        ..XbxEngineMediaRuntimeStats::default()
    };
    let closed = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Closed,
        ..XbxEngineMediaRuntimeStats::default()
    };
    let recovering = XbxEngineMediaRuntimeStats {
        session_phase: Some("recovering".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };
    let ramp_up = XbxEngineMediaRuntimeStats {
        session_phase: Some("ramp-up".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let startup_dto = build_xbxengine_stats(&test_snapshot(), Some(&startup));
    let steady_dto = build_xbxengine_stats(&test_snapshot(), Some(&steady));
    let degraded_dto = build_xbxengine_stats(&test_snapshot(), Some(&degraded));
    let failed_dto = build_xbxengine_stats(&test_snapshot(), Some(&failed));
    let closed_dto = build_xbxengine_stats(&test_snapshot(), Some(&closed));
    let recovering_dto = build_xbxengine_stats(&test_snapshot(), Some(&recovering));
    let ramp_up_dto = build_xbxengine_stats(&test_snapshot(), Some(&ramp_up));

    assert_eq!(
        startup_dto.stream_lifecycle_phase.as_deref(),
        Some("startup")
    );
    assert_eq!(steady_dto.stream_lifecycle_phase.as_deref(), Some("steady"));
    assert_eq!(
        degraded_dto.stream_lifecycle_phase.as_deref(),
        Some("degraded")
    );
    assert_eq!(failed_dto.stream_lifecycle_phase.as_deref(), Some("failed"));
    assert_eq!(closed_dto.stream_lifecycle_phase.as_deref(), Some("closed"));
    assert_eq!(
        recovering_dto.stream_lifecycle_phase.as_deref(),
        Some("recovering")
    );
    assert_eq!(
        ramp_up_dto.stream_lifecycle_phase.as_deref(),
        Some("ramp-up")
    );
}

#[test]
fn latest_decision_summary_is_driven_by_canonical_owner_contract() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_recovery_epoch: 3,
        transport_recovery_epoch_at_last_escalation: 3,
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(1234.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.latest_decision_summary.as_deref(),
        Some("owner:rebuilding-supply:transportAwaitRecoveryKeyframe")
    );
}

#[test]
fn display_supply_critical_uses_dedicated_stall_kind() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        video_owner_state: Some("supply-starved".to_string()),
        video_owner_reason: Some("displaySupplyCritical".to_string()),
        video_owner_source: Some("supply".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.stall_kind.as_deref(), Some("displaySupplyCritical"));
}

#[test]
fn supply_starved_owner_maps_stall_kind_to_display_supply_starved() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        video_owner_state: Some("supply-starved".to_string()),
        video_owner_reason: Some("supplyStarved".to_string()),
        video_owner_source: Some("steady".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.stall_kind.as_deref(), Some("displaySupplyStarved"));
}

#[test]
fn stable_serving_steady_stall_kind_is_none_not_recovering() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        video_owner_state: Some("stable-serving".to_string()),
        video_owner_reason: Some("steady".to_string()),
        video_owner_source: Some("steady".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.stall_kind.as_deref(), Some("none"));
}

#[test]
fn runtime_summary_includes_repair_probe_note_when_active() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(20.0),
        latest_video_host_present_time_ms: Some(30.0),
        video_present_submit_count_total: 1,
        direct_gaming_bitrate_band: Some("steady".to_string()),
        latest_video_repair_probe_observation: Some(crate::XbxEngineVideoRepairProbeObservation {
            observation_id: 1,
            phase: "packet".to_string(),
            classification: "repair-mime".to_string(),
            stream_id: "rtx-1".to_string(),
            stream_ssrc: 11,
            mime_type: "video/rtx".to_string(),
            payload_type: 97,
            clock_rate: 90_000,
            associated_ssrc: Some(42),
            associated_payload_type: Some(124),
            stream_packet_count: 8,
            observed_at_ms: 2_000.0,
        }),
        video_repair_probe_active_since_ms: Some(1_000.0),
        video_repair_probe_packet_count_total: 8,
        video_repair_probe_recovered_count_since_active: 3,
        video_repair_probe_late_recovered_count_since_active: 1,
        video_repair_probe_expired_count_since_active: 0,
        video_repair_probe_packet_gap_count_since_active: 2,
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    let runtime_summary = dto.runtime_summary.expect("runtime summary");

    assert!(runtime_summary.contains("repair:repair-mime:video/rtx:packet"));
    assert!(runtime_summary.contains("id=rtx-1"));
    assert!(runtime_summary.contains("rec=3"));
    assert!(runtime_summary.contains("exp=0"));
}

#[test]
fn runtime_summary_includes_rtx_reinject_note_when_present() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(20.0),
        latest_video_host_present_time_ms: Some(30.0),
        video_present_submit_count_total: 1,
        latest_video_rtx_reinject_observation: Some(crate::XbxEngineVideoRtxReinjectObservation {
            stage: "adapterResolved".to_string(),
            primary_ssrc: 10,
            repair_ssrc: 20,
            sequence_number: 18_894,
            rtp_timestamp: 123,
            pending_queue_len: 0,
            native_sequence_number: None,
            matched_head_gap: true,
            matched_nack_range: true,
            matched_pending_gap: true,
            matched_gap_sequence: Some(18_894),
            matched_nack_first_sequence: Some(18_894),
            matched_nack_last_sequence: Some(18_894),
            observed_at_ms: 1_000.0,
        }),
        video_rtx_reinject_head_match_count_total: 2,
        video_rtx_reinject_range_match_count_total: 1,
        video_rtx_reinject_miss_count_total: 1,
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    let runtime_summary = dto.runtime_summary.expect("runtime summary");

    assert!(runtime_summary.contains("reinject:stage=adapterResolved seq=18894"));
    assert!(runtime_summary.contains("headMatch=true"));
    assert!(runtime_summary.contains("rangeMatch=true"));
    assert!(runtime_summary.contains("headHitRate=0.500"));
}

#[test]
fn owner_contract_projection_reads_canonical_runtime_owner_fields() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        message_handshake_acked_at_ms: Some(now_ms - 120.0),
        control_ready_at_ms: Some(now_ms - 110.0),
        latest_video_host_present_time_ms: Some(now_ms - 30.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 30.0),
        video_present_submit_count_total: 2,
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("timelineReferenceBroken".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 10.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.recovery_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        dto.recovery_owner_reason.as_deref(),
        Some("timelineReferenceBroken")
    );
    assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
    assert_eq!(dto.video_owner_observed_at_ms, Some(now_ms - 10.0));
    assert_eq!(dto.video_health.as_deref(), Some("recovering"));
    assert_eq!(
        dto.primary_issue_chain.as_deref(),
        Some("recovery:timelineReferenceBroken")
    );
    assert_eq!(
        dto.latest_decision_summary.as_deref(),
        Some("owner:rebuilding-supply:timelineReferenceBroken")
    );
    // coupling 字段仅保留为辅助观测，不参与 owner 语义。
}

#[test]
fn owner_contract_falls_back_to_runtime_state_primary_view() {
    let stats = XbxEngineMediaRuntimeStats {
        session_phase: Some("recovering".to_string()),
        recovery_diagnosis: Some("transportAwaitRecoveryKeyframe".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(
        dto.recovery_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        dto.recovery_owner_reason.as_deref(),
        Some("transportAwaitRecoveryKeyframe")
    );
    assert_eq!(
        dto.video_owner_source.as_deref(),
        Some("runtime-recovering")
    );
    assert_eq!(
        dto.latest_decision_summary.as_deref(),
        Some("owner:rebuilding-supply:transportAwaitRecoveryKeyframe")
    );
}

#[test]
fn build_stats_uses_handshaking_phase_before_handshake_ack() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(dto.session_phase.as_deref(), Some("handshaking"));
    assert_eq!(dto.stream_lifecycle_phase.as_deref(), Some("startup"));
    assert_eq!(dto.video_health, None);
    assert_eq!(dto.primary_issue_chain, None);
}

#[test]
fn build_stats_uses_priming_phase_before_first_present() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(20.0),
        latest_video_packet_arrival_time_ms: Some(30.0),
        latest_video_decode_ok_time_ms: Some(35.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(dto.session_phase.as_deref(), Some("priming"));
    assert_eq!(dto.stream_lifecycle_phase.as_deref(), Some("startup"));
    assert_eq!(dto.video_health, None);
    assert_eq!(dto.primary_issue_chain, None);
}

#[test]
fn build_stats_keeps_priming_when_only_submit_count_exists_without_host_present() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(20.0),
        latest_video_packet_arrival_time_ms: Some(30.0),
        latest_video_decode_ok_time_ms: Some(35.0),
        video_present_submit_count_total: 120,
        video_present_fps: 0.0,
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(dto.session_phase.as_deref(), Some("priming"));
    assert_eq!(dto.recovery_owner_state, None);
    assert_eq!(dto.video_health, None);
}

#[test]
fn build_stats_only_turns_healthy_after_first_present() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 100.0),
        control_ready_at_ms: Some(now_ms - 90.0),
        latest_video_host_present_time_ms: Some(now_ms - 20.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 20.0),
        video_present_submit_count_total: 1,
        video_present_fps: 60.0,
        video_owner_state: Some("stable-serving".to_string()),
        video_owner_reason: Some("steady".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 5.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(dto.session_phase.as_deref(), Some("steady"));
    assert_eq!(dto.stream_lifecycle_phase.as_deref(), Some("steady"));
    assert_eq!(dto.video_health.as_deref(), Some("healthy"));
    assert_eq!(dto.recovery_owner_state.as_deref(), Some("stable-serving"));
    assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
    assert_eq!(dto.primary_issue_chain.as_deref(), Some("steady:healthy"));
}

#[test]
fn build_stats_projects_ramp_up_without_overwriting_legacy_session_phase() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        transport_recovery_episode_active: true,
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 100.0),
        control_ready_at_ms: Some(now_ms - 90.0),
        latest_video_host_present_time_ms: Some(now_ms - 20.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 20.0),
        video_present_submit_count_total: 1,
        video_present_fps: 60.0,
        video_owner_state: Some("stable-serving".to_string()),
        video_owner_reason: Some("steady".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 5.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(dto.session_phase.as_deref(), Some("steady"));
    assert_eq!(dto.stream_lifecycle_phase.as_deref(), Some("ramp-up"));
}

#[test]
fn build_stats_projects_degraded_without_overwriting_legacy_session_phase() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 100.0),
        control_ready_at_ms: Some(now_ms - 90.0),
        latest_video_host_present_time_ms: Some(now_ms - 20.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 20.0),
        video_present_submit_count_total: 1,
        video_present_fps: 60.0,
        video_owner_state: Some("degraded-serving".to_string()),
        video_owner_reason: Some("degradedSteady".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 5.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(dto.session_phase.as_deref(), Some("steady"));
    assert_eq!(dto.stream_lifecycle_phase.as_deref(), Some("degraded"));
}

#[test]
fn build_stats_reports_recovering_after_first_present_when_output_turns_stale() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("recovering".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 1_000.0),
        control_ready_at_ms: Some(now_ms - 990.0),
        latest_video_host_present_time_ms: Some(now_ms - 800.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 800.0),
        video_present_submit_count_total: 1,
        recovery_diagnosis: Some("adapterIdleTimeout".to_string()),
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("adapterIdleTimeout".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 10.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

    assert_eq!(dto.session_phase.as_deref(), Some("recovering"));
    assert_eq!(dto.stream_lifecycle_phase.as_deref(), Some("recovering"));
    assert_eq!(dto.video_health.as_deref(), Some("recovering"));
    assert_eq!(
        dto.primary_issue_chain.as_deref(),
        Some("recovery:adapterIdleTimeout")
    );
}

#[test]
fn build_stats_prioritizes_display_supply_starved_when_no_pending_and_present_age_is_stale() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 4_000.0),
        control_ready_at_ms: Some(now_ms - 3_900.0),
        latest_video_host_present_time_ms: Some(now_ms - 2_200.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 1_600.0),
        video_present_submit_count_total: 120,
        video_present_fps: 1.0,
        host_no_pending_pressure_level: Some("critical".to_string()),
        host_no_pending_streak: 1_280,
        host_no_pending_max_streak: 1_500,
        video_owner_state: Some("supply-starved".to_string()),
        video_owner_reason: Some("supply-starved".to_string()),
        video_owner_source: Some("supply".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 10.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_health.as_deref(), Some("displaySupplyStarved"));
    assert_eq!(dto.recovery_owner_state.as_deref(), Some("supply-starved"));
    assert_eq!(dto.recovery_owner_reason.as_deref(), Some("supply-starved"));
    assert_eq!(dto.video_owner_source.as_deref(), Some("supply"));
    assert_eq!(
        dto.primary_issue_chain.as_deref(),
        Some("display:supply-starved")
    );
}

#[test]
fn build_stats_projects_host_cadence_epoch_fields() {
    let stats = XbxEngineMediaRuntimeStats {
        host_display_tick_epoch: 128,
        video_present_epoch: 96,
        host_cadence_phase: Some("starved".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.host_display_tick_epoch, Some(128));
    assert_eq!(dto.video_present_epoch, Some(96));
    assert_eq!(dto.host_cadence_phase.as_deref(), Some("starved"));
}

#[test]
fn build_stats_ignores_stale_recovery_diagnosis_when_output_is_fresh() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        recovery_diagnosis: Some("adapterIdleTimeout".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 80.0),
        control_ready_at_ms: Some(now_ms - 70.0),
        latest_video_host_present_time_ms: Some(now_ms - 35.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 35.0),
        video_present_submit_count_total: 2,
        video_present_fps: 58.0,
        host_no_pending_pressure_level: Some("normal".to_string()),
        host_no_pending_streak: 1,
        video_owner_state: Some("stable-serving".to_string()),
        video_owner_reason: Some("steady".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 10.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_health.as_deref(), Some("healthy"));
    assert_eq!(dto.primary_issue_chain.as_deref(), Some("steady:healthy"));
}

#[test]
fn build_stats_prioritizes_recent_timeline_recovering_over_healthy_summary() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 80.0),
        control_ready_at_ms: Some(now_ms - 70.0),
        latest_video_host_present_time_ms: Some(now_ms - 35.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 35.0),
        video_present_submit_count_total: 2,
        video_present_fps: 58.0,
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 10.0),
        latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 7,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: Some(crate::XbxEngineVideoTimelineFrameSnapshot {
                state: "closed".to_string(),
                frame_rtp_timestamp: Some(123),
                is_keyframe: Some(false),
                frame_importance: Some("unknown".to_string()),
                close_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
                observed_at_ms: now_ms - 20.0,
            }),
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
                observed_at_ms: now_ms - 20.0,
            },
            observed_at_ms: now_ms - 20.0,
        }),
        latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
            observation_id: 1,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            action: "requestDecoderReset".to_string(),
            recovery_stage: "rebuilding-supply".to_string(),
            recovery_chain_value: "anchor".to_string(),
            recovery_failure_cost: "high".to_string(),
            recovery_window_source: "transport-await-window".to_string(),
            observed_at_ms: now_ms - 10.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_health.as_deref(), Some("recovering"));
    assert_eq!(
        dto.recovery_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        dto.recovery_owner_reason.as_deref(),
        Some("inspectionRejectInvalidSliceHeader")
    );
    assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
    assert_eq!(
        dto.primary_issue_chain.as_deref(),
        Some("recovery:inspectionRejectInvalidSliceHeader")
    );
    assert_eq!(
        dto.latest_decision_summary.as_deref(),
        Some("owner:rebuilding-supply:inspectionRejectInvalidSliceHeader")
    );
}

#[test]
fn build_stats_prioritizes_recent_timeline_broken_over_steady_healthy() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 80.0),
        control_ready_at_ms: Some(now_ms - 70.0),
        latest_video_host_present_time_ms: Some(now_ms - 40.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 35.0),
        video_present_submit_count_total: 3,
        video_present_fps: 60.0,
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("cloudHighRttLowValueAdmission".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 10.0),
        latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 8,
            source_event: "nack-observation".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "expired".to_string(),
                sequence: Some(38022),
                frame_rtp_timestamp: Some(456),
                frame_importance: Some("delta".to_string()),
                observed_at_ms: now_ms - 15.0,
            }),
            frame: Some(crate::XbxEngineVideoTimelineFrameSnapshot {
                state: "gap-present".to_string(),
                frame_rtp_timestamp: Some(456),
                is_keyframe: Some(false),
                frame_importance: Some("delta".to_string()),
                close_reason: None,
                observed_at_ms: now_ms - 15.0,
            }),
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("cloudHighRttLowValueAdmission".to_string()),
                observed_at_ms: now_ms - 15.0,
            },
            observed_at_ms: now_ms - 15.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_health.as_deref(), Some("recovering"));
    assert_eq!(
        dto.recovery_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        dto.recovery_owner_reason.as_deref(),
        Some("cloudHighRttLowValueAdmission")
    );
    assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
    assert_eq!(
        dto.primary_issue_chain.as_deref(),
        Some("recovery:cloudHighRttLowValueAdmission")
    );
    assert_eq!(
        dto.latest_decision_summary.as_deref(),
        Some("owner:rebuilding-supply:cloudHighRttLowValueAdmission")
    );
}

#[test]
fn build_stats_owner_contract_prefers_canonical_owner_over_coupling_signals() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        session_phase: Some("steady".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(now_ms - 2000.0),
        control_ready_at_ms: Some(now_ms - 1900.0),
        latest_video_host_present_time_ms: Some(now_ms - 1700.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 1700.0),
        video_present_submit_count_total: 64,
        host_no_pending_pressure_level: Some("critical".to_string()),
        host_no_pending_streak: 2048,
        video_renderer_stalled: Some(true),
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
        video_owner_source: Some("anchor".to_string()),
        video_owner_observed_at_ms: Some(now_ms - 10.0),
        latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 9,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
                observed_at_ms: now_ms - 15.0,
            },
            observed_at_ms: now_ms - 15.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_health.as_deref(), Some("recovering"));
    assert_eq!(
        dto.recovery_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
    assert_eq!(
        dto.primary_issue_chain.as_deref(),
        Some("recovery:inspectionRejectInvalidSliceHeader")
    );
    assert_eq!(
        dto.latest_decision_summary.as_deref(),
        Some("owner:rebuilding-supply:inspectionRejectInvalidSliceHeader")
    );
}

#[test]
fn build_stats_falls_back_to_runtime_strategy_profile() {
    let stats = XbxEngineMediaRuntimeStats {
        baseline_remote_profile: Some("cloudGaming".to_string()),
        dynamic_remote_subprofile: Some("cloudHighRtt".to_string()),
        effective_remote_profile_label: Some("cloudGaming+cloudHighRtt".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.recovery_strategy_profile.as_deref(),
        Some("cloudGaming")
    );
    assert_eq!(
        dto.remote_profile_effective_label.as_deref(),
        Some("cloudGaming+cloudHighRtt")
    );
}

#[test]
fn build_stats_recovery_strategy_profile_follows_runtime_strategy_profile() {
    let stats = XbxEngineMediaRuntimeStats {
        session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
        baseline_remote_profile: Some("relayGaming".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.recovery_strategy_profile.as_deref(),
        Some("relayGaming")
    );
    assert_eq!(dto.remote_profile_baseline.as_deref(), Some("relayGaming"));
}

#[test]
fn runtime_summary_profile_slot_prefers_runtime_profile_over_transport_policy() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        baseline_remote_profile: Some("relayGaming".to_string()),
        effective_remote_profile_label: Some("relayGaming+steady".to_string()),
        session_phase: Some("steady".to_string()),
        direct_gaming_bitrate_band: Some("steady".to_string()),
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(20.0),
        latest_video_host_present_time_ms: Some(30.0),
        video_present_submit_count_total: 1,
        video_owner_state: Some("stable-serving".to_string()),
        video_owner_reason: Some("steady".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.runtime_summary.as_deref(),
        Some("relayGaming+steady/steady/steady/stable-serving/healthy")
    );
}

#[test]
fn runtime_summary_profile_slot_does_not_fallback_to_transport_policy_only() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_policy_profile: Some("cloud".to_string()),
        message_handshake_acked_at_ms: Some(10.0),
        control_ready_at_ms: Some(20.0),
        latest_video_host_present_time_ms: Some(30.0),
        video_present_submit_count_total: 1,
        direct_gaming_bitrate_band: Some("steady".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.runtime_summary.as_deref(),
        Some("homeLanGaming+steady/steady/steady/stable-serving/healthy")
    );
}

#[test]
fn build_stats_prefers_owner_reason_for_recovery_diagnosis() {
    let stats = XbxEngineMediaRuntimeStats {
        recovery_diagnosis: Some("transportExpiredDeadline".to_string()),
        video_owner_state: Some("rebuilding-supply".to_string()),
        video_owner_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.recovery_diagnosis.as_deref(),
        Some("inspectionRejectInvalidSliceHeader")
    );
}

#[test]
fn audio_inbound_bitrate_is_estimated_from_audio_bytes_when_playback_is_absent() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        first_audio_packet_arrival_time_ms: Some(now_ms - 2_000.0),
        latest_audio_packet_arrival_time_ms: Some(now_ms - 120.0),
        inbound_audio_bytes_total: 250_000,
        inbound_video_bitrate_kbps: Some(16_000.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert!(dto.inbound_audio_bitrate_kbps.unwrap_or(0.0) > 0.0);
    assert!(
        dto.inbound_bitrate_kbps.unwrap_or(0.0) >= dto.inbound_video_bitrate_kbps.unwrap_or(0.0)
    );
}

#[test]
fn video_inbound_bitrate_does_not_fallback_to_media_ingress_bytes_when_transport_stats_are_zero() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        first_video_packet_arrival_time_ms: Some(now_ms - 2_000.0),
        latest_video_packet_arrival_time_ms: Some(now_ms - 16.0),
        inbound_video_bytes_total: 2_000_000,
        inbound_video_bitrate_kbps: Some(0.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.inbound_video_bitrate_kbps, None);
    assert_eq!(dto.video_actual_bitrate_kbps, None);
    assert_eq!(
        dto.actual_video_bitrate_source.as_deref(),
        Some("unavailable")
    );
}

#[test]
fn total_inbound_bitrate_prefers_video_plus_audio_components() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        inbound_bitrate_kbps: Some(128.0),
        inbound_video_bitrate_kbps: Some(8_800.0),
        inbound_audio_bitrate_kbps: Some(160.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.inbound_video_bitrate_kbps, Some(8_800.0));
    assert_eq!(dto.inbound_audio_bitrate_kbps, Some(160.0));
    assert_eq!(dto.inbound_bitrate_kbps, Some(8_960.0));
}

#[test]
fn bwe_and_twcc_semantic_fields_are_projected_explicitly() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        video_remb_bps: Some(25_000_000),
        latest_video_bwe_observation: Some(crate::XbxEngineVideoBweObservation {
            observation_id: 7,
            mode: "twcc-gcc".to_string(),
            decision_reason: "twcc-gcc-cloud-stable-ramp".to_string(),
            target_remb_kbps: 25_000,
            observed_remb_kbps: Some(23_000),
            actual_video_bitrate_kbps: 21_500.0,
            loss_ratio: 0.01,
            rtt_ms: Some(82.0),
            transport_path: Some("Direct".to_string()),
            twcc_feedback_interval_ms: Some(80.0),
            twcc_observed_packet_count: Some(120),
            twcc_covered_sequence_span: Some(120),
            twcc_receive_bitrate_kbps: Some(22_800.0),
            twcc_delivery_ratio: Some(0.99),
            twcc_loss_ratio: Some(0.01),
            observed_at_ms: 1.0,
        }),
        latest_video_twcc_observation: Some(crate::XbxEngineVideoTwccObservation {
            observation_id: 8,
            source: "local-feedback".to_string(),
            feedback_packet_count: 3,
            covered_sequence_start: 100,
            covered_sequence_end: 220,
            covered_sequence_span: 120,
            observed_packet_count: 120,
            observed_byte_count: 340_000,
            coverage_ratio: Some(1.0),
            ledger_hit_ratio: Some(1.0),
            feedback_interval_ms: Some(80.0),
            arrival_span_ms: Some(70.0),
            receive_bitrate_kbps: Some(22_800.0),
            twcc_sample_valid: true,
            twcc_invalid_reason: None,
            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 0.99,
            packet_loss_ratio: 0.01,
            observed_at_ms: 2.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_bwe_mode.as_deref(), Some("twcc-gcc"));
    assert_eq!(dto.video_target_remb_kbps, Some(25_000));
    assert_eq!(dto.video_observed_remb_kbps, Some(23_000));
    assert_eq!(dto.video_actual_bitrate_kbps, None);
    assert_eq!(dto.video_twcc_receive_bitrate_kbps, Some(22_800.0));
    assert_eq!(dto.video_twcc_loss_ratio, Some(0.01));
    assert_eq!(dto.video_twcc_delivery_ratio, Some(0.99));
    assert_eq!(dto.video_twcc_feedback_interval_ms, Some(80.0));
    assert_eq!(
        dto.actual_video_bitrate_source.as_deref(),
        Some("unavailable")
    );
    assert_eq!(
        dto.twcc_observation_state.as_deref(),
        Some("local-feedback")
    );
}

#[test]
fn actual_video_bitrate_uses_transport_metrics_when_local_twcc_missing() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        inbound_video_bitrate_kbps: Some(8_600.0),
        latest_video_twcc_observation: Some(crate::XbxEngineVideoTwccObservation {
            observation_id: 8,
            source: "remote-rtcp".to_string(),
            feedback_packet_count: 3,
            covered_sequence_start: 100,
            covered_sequence_end: 220,
            covered_sequence_span: 120,
            observed_packet_count: 120,
            observed_byte_count: 340_000,
            coverage_ratio: Some(1.0),
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(80.0),
            arrival_span_ms: Some(70.0),
            receive_bitrate_kbps: Some(22_800.0),
            twcc_sample_valid: true,
            twcc_invalid_reason: None,
            quality: crate::XbxEngineTwccObservationQuality::RemoteObserved,
            delivery_ratio: 0.99,
            packet_loss_ratio: 0.01,
            observed_at_ms: 2.0,
        }),
        latest_twcc_remote_stream_observation: Some(crate::XbxEngineTwccRemoteStreamObservation {
            observation_id: 11,
            ssrc: 42,
            mime_type: "video/H264".to_string(),
            twcc_ext_id: Some(7),
            header_extensions: vec!["transport-cc#7".to_string()],
            rtcp_feedback: vec!["transport-cc:".to_string()],
            observed_at_ms: 3.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_actual_bitrate_kbps, Some(8_600.0));
    assert_eq!(
        dto.actual_video_bitrate_source.as_deref(),
        Some("transport-metrics")
    );
    assert_eq!(
        dto.twcc_observation_state.as_deref(),
        Some("remote-observed")
    );
}

#[test]
fn transport_details_fields_are_projected_from_runtime_stats() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        transport_path: Some("Direct (host->srflx)".to_string()),
        transport_candidate_pair: Some("host->srflx".to_string()),
        transport_protocol: Some("UDP".to_string()),
        transport_address_family: Some("ipv4".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.transport_path.as_deref(), Some("Direct (host->srflx)"));
    assert_eq!(dto.transport_candidate_pair.as_deref(), Some("host->srflx"));
    assert_eq!(dto.transport_protocol.as_deref(), Some("UDP"));
    assert_eq!(dto.transport_address_family.as_deref(), Some("ipv4"));
}

#[test]
fn actual_video_bitrate_uses_transport_metrics_when_local_twcc_is_guarded() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        inbound_video_bitrate_kbps: Some(8_600.0),
        latest_video_twcc_observation: Some(crate::XbxEngineVideoTwccObservation {
            observation_id: 9,
            source: "local-feedback".to_string(),
            feedback_packet_count: 3,
            covered_sequence_start: 100,
            covered_sequence_end: 220,
            covered_sequence_span: 120,
            observed_packet_count: 6,
            observed_byte_count: 0,
            coverage_ratio: Some(0.05),
            ledger_hit_ratio: Some(0.0),
            feedback_interval_ms: Some(900.0),
            arrival_span_ms: Some(120.0),
            receive_bitrate_kbps: None,
            twcc_sample_valid: false,
            twcc_invalid_reason: Some("missing-byte-ledger|sample-too-small".to_string()),
            quality: crate::XbxEngineTwccObservationQuality::Delayed,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_actual_bitrate_kbps, Some(8_600.0));
    assert_eq!(
        dto.actual_video_bitrate_source.as_deref(),
        Some("transport-metrics")
    );
}

#[test]
fn twcc_state_marks_missing_header_extension_when_feedback_chain_has_not_started() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        latest_rtc_builder_observation: Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 1_000.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 1.0,
        }),
        latest_twcc_remote_stream_observation: Some(crate::XbxEngineTwccRemoteStreamObservation {
            observation_id: 2,
            ssrc: 42,
            mime_type: "video/H264".to_string(),
            twcc_ext_id: Some(7),
            header_extensions: vec!["transport-cc#7".to_string()],
            rtcp_feedback: vec!["transport-cc:".to_string()],
            observed_at_ms: 2.0,
        }),
        latest_twcc_extension_observation: Some(crate::XbxEngineTwccExtensionObservation {
            observation_id: 3,
            state: "missing".to_string(),
            ssrc: 42,
            sequence_number: 99,
            expected_ext_id: 7,
            packet_seen_count: 1,
            missing_count: 1,
            observed_at_ms: 3.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.video_actual_bitrate_kbps, None);
    assert_eq!(
        dto.actual_video_bitrate_source.as_deref(),
        Some("unavailable")
    );
    assert_eq!(
        dto.twcc_observation_state.as_deref(),
        Some("missing-header-extension")
    );
}

#[test]
fn twcc_state_stays_builder_configured_when_only_audio_remote_binding_exists() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_state: XbxEngineTransportStateDto::Connected,
        latest_rtc_builder_observation: Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 1_000.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 1.0,
        }),
        latest_twcc_remote_stream_observation: Some(crate::XbxEngineTwccRemoteStreamObservation {
            observation_id: 2,
            ssrc: 99,
            mime_type: "audio/opus".to_string(),
            twcc_ext_id: Some(7),
            header_extensions: vec!["transport-cc#7".to_string()],
            rtcp_feedback: vec!["transport-cc:".to_string()],
            observed_at_ms: 2.0,
        }),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.twcc_observation_state.as_deref(),
        Some("builder-configured")
    );
}

#[test]
fn stats_expose_cloud_startup_dynamic_subprofile() {
    let stats = XbxEngineMediaRuntimeStats {
        session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
        session_phase: Some("startup".to_string()),
        direct_gaming_bitrate_band: Some("startupLow".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.remote_profile_baseline.as_deref(), Some("cloudGaming"));
    assert_eq!(dto.remote_profile_dynamic.as_deref(), Some("cloudStartup"));
    assert_eq!(
        dto.remote_profile_effective_label.as_deref(),
        Some("cloudGaming+cloudStartup")
    );
}

#[test]
fn stats_expose_display_constrained_dynamic_subprofile() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let stats = XbxEngineMediaRuntimeStats {
        session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
        host_no_pending_pressure_level: Some("critical".to_string()),
        latest_video_decode_ok_time_ms: Some(now_ms - 1_500.0),
        latest_video_host_present_time_ms: Some(now_ms - 1_500.0),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(
        dto.remote_profile_dynamic.as_deref(),
        Some("displayConstrained")
    );
    assert_eq!(
        dto.remote_profile_effective_label.as_deref(),
        Some("cloudGaming+displayConstrained")
    );
}

#[test]
fn stats_prioritize_runtime_remote_profile_facts_when_present() {
    let stats = XbxEngineMediaRuntimeStats {
        session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
        session_phase: Some("startup".to_string()),
        direct_gaming_bitrate_band: Some("startupLow".to_string()),
        baseline_remote_profile: Some("relayGaming".to_string()),
        dynamic_remote_subprofile: Some("steady".to_string()),
        effective_remote_profile_label: Some("relayGaming+steady".to_string()),
        ..XbxEngineMediaRuntimeStats::default()
    };

    let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
    assert_eq!(dto.remote_profile_baseline.as_deref(), Some("relayGaming"));
    assert_eq!(dto.remote_profile_dynamic.as_deref(), Some("steady"));
    assert_eq!(
        dto.remote_profile_effective_label.as_deref(),
        Some("relayGaming+steady")
    );
}
