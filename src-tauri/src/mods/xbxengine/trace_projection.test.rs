use super::*;
use crate::mods::runtime_trace::RuntimeTraceRecorder;
use serde_json::{json, Value};
use std::fs;

fn test_stats(payload: serde_json::Value) -> XbxEngineStatsDto {
    serde_json::from_value(payload).expect("valid stats dto")
}

fn read_trace_lines(recorder: &RuntimeTraceRecorder) -> Vec<Value> {
    let contents =
        fs::read_to_string(recorder.path().expect("trace path")).expect("trace contents");
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn find_event_payload(entries: &[Value], event: &str) -> Value {
    entries
        .iter()
        .find(|entry| entry["event"] == event)
        .map(|entry| entry["payload"].clone())
        .unwrap_or_else(|| panic!("event not found: {event}"))
}

fn event_payloads(entries: &[Value], event: &str) -> Vec<Value> {
    entries
        .iter()
        .filter(|entry| entry["event"] == event)
        .map(|entry| entry["payload"].clone())
        .collect()
}

fn has_event(entries: &[Value], event: &str) -> bool {
    entries.iter().any(|entry| entry["event"] == event)
}

#[test]
fn build_observability_snapshot_includes_source_state_and_build_fingerprint() {
    let stats = test_stats(json!({
        "resolution": "2560x1440",
        "rtt": "81.0ms",
        "fps": 60.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "1.0ms",
        "br": "8.5Mbps",
        "decode": "",
        "inbound_bitrate_kbps": 8600.0,
        "inbound_video_bitrate_kbps": 8400.0,
        "inbound_audio_bitrate_kbps": 200.0,
        "latest_audio_playout_time_ms": 1774405700123.0,
        "audio_playout_latency_ms": 41.5,
        "audio_video_playout_delta_ms": 8.5,
        "actual_video_bitrate_source": "transport-metrics",
        "video_actual_bitrate_kbps": 8400.0,
        "video_twcc_receive_bitrate_kbps": 22800.0,
        "video_twcc_loss_ratio": 0.01,
        "video_twcc_delivery_ratio": 0.99,
        "video_twcc_feedback_interval_ms": 80.0,
        "twcc_observation_state": "missing-local-feedback",
        "build_fingerprint": {
            "gitCommitShort": "abc1234",
            "workspaceDirty": true,
            "buildTimestampUnixMs": "1774405700000",
            "cargoProfile": "debug",
            "defaultFeedbackIntervalMs": 1000,
            "effectiveFeedbackIntervalMs": 80,
            "controlledTwccRegistry": true
        }
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(snapshot["bitrate"]["actualVideoKbps"], 8400.0);
    assert_eq!(snapshot["audio"]["audioPlayoutLatencyMs"], 41.5);
    assert_eq!(snapshot["audio"]["audioVideoPlayoutDeltaMs"], 8.5);
    assert_eq!(
        snapshot["bwe"]["actualVideoBitrateSource"],
        "transport-metrics"
    );
    assert_eq!(snapshot["bwe"]["actualVideoKbps"], 8400.0);
    assert_eq!(snapshot["twcc"]["state"], "missing-local-feedback");
    assert_eq!(snapshot["buildFingerprint"]["gitCommitShort"], "abc1234");
}

#[test]
fn build_observability_snapshot_includes_latest_frame_recovery() {
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "actual_video_bitrate_source": "unavailable",
        "video_actual_bitrate_kbps": 1019.4,
        "latest_video_frame_recovery_observation": {
            "observation_id": 77,
            "action": "ledgerConsume",
            "frame_rtp_timestamp": 123456789,
            "frame_playout_deadline_at_ms": 4567.0,
            "frame_recovery_disposition": "unrecoverable-reference-chain",
            "frame_unrecoverable_reason": "referenceChainUnrecoverable",
            "frame_budget": {
                "recovery_stage": "awaiting-anchor",
                "chain_value": "supply",
                "rtt_slack": "tight",
                "failure_cost": "chain-broken",
                "window_source": "recovery"
            },
            "observed_at_ms": 1234.0
        }
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(snapshot["bitrate"]["actualVideoKbps"], 1019.4);
    assert_eq!(
        snapshot["bitrate"]["actualVideoBitrateSource"],
        "unavailable"
    );
    assert_eq!(
        snapshot["latest"]["frameRecovery"]["action"],
        "ledgerConsume"
    );
    assert_eq!(
        snapshot["latest"]["frameRecovery"]["frame_recovery_disposition"],
        "unrecoverable-reference-chain"
    );
    assert_eq!(
        snapshot["latest"]["frameRecovery"]["frame_unrecoverable_reason"],
        "referenceChainUnrecoverable"
    );
    assert_eq!(
        snapshot["latest"]["frameRecovery"]["frame_budget"]["recovery_stage"],
        "awaiting-anchor"
    );
    assert_eq!(
        snapshot["latest"]["frameRecovery"]["frame_budget"]["chain_value"],
        "supply"
    );
    assert_eq!(snapshot["frameBudget"]["source"], "frameRecovery");
    assert_eq!(snapshot["frameBudget"]["failureCost"], "chain-broken");
}

#[test]
fn build_observability_snapshot_includes_latest_keyframe_episode() {
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 91,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "decoded",
            "lifecycle_phase": "decoded",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "first_keyframe_packet_at_ms": 1400.0,
            "first_keyframe_decoded_at_ms": 1420.0,
            "response_rtp_timestamp": 123456789,
            "response_frame_seq": 77,
            "response_verdict": "on-time"
        },
        "recent_keyframe_request_episodes": [
            {
                "episode_id": 90,
                "request_reason": "ingressWaitKeyframe",
                "request_kind": "pli",
                "status": "missed",
                "lifecycle_phase": "failure",
                "requested_at_ms": 500.0,
                "sent_at_ms": 520.0,
                "deadline_at_ms": 800.0,
                "response_verdict": "missed"
            }
        ]
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["episode_id"],
        91
    );
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["request_reason"],
        "transportAwaitRecoveryAnchor"
    );
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["response_verdict"],
        "on-time"
    );
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["lifecycle_phase"],
        "decoded"
    );
    assert_eq!(
        snapshot["latest"]["recentKeyframeRequestEpisodes"][0]["episode_id"],
        90
    );
    assert_eq!(
        snapshot["latest"]["recentKeyframeRequestEpisodes"][0]["lifecycle_phase"],
        "failure"
    );
}

#[test]
fn build_observability_snapshot_includes_suppression_family_and_health_split() {
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "video_health": "displaySupplyStarved",
        "chain_health": "healthy",
        "presentation_health": "displaySupplyStarved",
        "latest_keyframe_request_episode": {
            "episode_id": 91,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "deferred",
            "status_detail": "transport-suppressed",
            "requested_at_ms": 1200.0,
            "deadline_at_ms": 2160.0,
            "transport_detail": "coalesced:keyframeInFlight",
            "response_verdict": "pending",
            "family_id": "transportAwaitRecoveryAnchor:pli",
            "owner_episode_id": 88,
            "suppress_duration_ms": 240.0,
            "release_reason": "ownerEpisodeSucceeded"
        }
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(snapshot["recovery"]["videoHealth"], "displaySupplyStarved");
    assert_eq!(snapshot["recovery"]["chainHealth"], "healthy");
    assert_eq!(
        snapshot["recovery"]["presentationHealth"],
        "displaySupplyStarved"
    );
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["family_id"],
        "transportAwaitRecoveryAnchor:pli"
    );
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["owner_episode_id"],
        88
    );
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["suppress_duration_ms"],
        240.0
    );
    assert_eq!(
        snapshot["latest"]["keyframeRequestEpisode"]["release_reason"],
        "ownerEpisodeSucceeded"
    );
}

#[test]
fn build_observability_snapshot_includes_latest_h264_inspection() {
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_h264_inspection_observation": {
            "observation_id": 2194672423u64,
            "frame_rtp_timestamp": 2194672423u32,
            "nal_types": ["SeqParameterSet", "PicParameterSet", "SliceLayerWithoutPartitioningIdr"],
            "has_inband_sps": true,
            "has_inband_pps": true,
            "committed_sps_present": false,
            "committed_pps_present": false,
            "slice_headers_valid": false,
            "delta_continuation_ready": false,
            "parameter_sets_changed": true,
            "config_changed": true,
            "is_idr": true,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "bootstrapMissingSps",
            "admission_accepted": false,
            "observed_at_ms": 1400.0
        }
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(
        snapshot["latest"]["h264Inspection"]["frameRtpTimestamp"],
        2194672423u32
    );
    assert_eq!(snapshot["latest"]["h264Inspection"]["hasInbandSps"], true);
    assert_eq!(
        snapshot["latest"]["h264Inspection"]["committedSpsPresent"],
        false
    );
    assert_eq!(
        snapshot["latest"]["h264Inspection"]["bootstrapRejectReason"],
        "bootstrapMissingSps"
    );
    assert_eq!(
        snapshot["latest"]["h264Inspection"]["deltaContinuationReady"],
        false
    );
    assert_eq!(
        snapshot["latest"]["h264Inspection"]["admissionAccepted"],
        false
    );
    assert_eq!(
        snapshot["latest"]["h264Inspection"]["linkedEpisodeId"],
        Value::Null
    );
}

#[test]
fn build_observability_snapshot_includes_latest_rtcp_send_failure() {
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_rtcp_send_failure_time_ms": 1500.0,
        "latest_video_rtcp_send_failure_reason": "rtcp-write-failed"
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(
        snapshot["latest"]["rtcpSendFailure"]["observedAtMs"],
        1500.0
    );
    assert_eq!(
        snapshot["latest"]["rtcpSendFailure"]["reason"],
        "rtcp-write-failed"
    );
}

#[test]
fn build_observability_snapshot_projects_latest_twcc_sample_gate_fields() {
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "twcc_observation_state": "unavailable",
        "latest_video_twcc_observation": {
            "observation_id": 19,
            "source": "local-feedback",
            "feedback_packet_count": 1,
            "covered_sequence_start": 1,
            "covered_sequence_end": 8,
            "covered_sequence_span": 8,
            "observed_packet_count": 8,
            "observed_byte_count": 0,
            "coverage_ratio": 1.0,
            "ledger_hit_ratio": 0.0,
            "feedback_interval_ms": 600.0,
            "arrival_span_ms": 100.0,
            "receive_bitrate_kbps": null,
            "twcc_sample_valid": false,
            "twcc_invalid_reason": "missing-byte-ledger|interval-too-long:600.0",
            "quality": "delayed",
            "delivery_ratio": 1.0,
            "packet_loss_ratio": 0.0,
            "observed_at_ms": 1234.0
        }
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(snapshot["twcc"]["sampleValid"], false);
    assert_eq!(
        snapshot["twcc"]["invalidReason"],
        "missing-byte-ledger|interval-too-long:600.0"
    );
    assert_eq!(snapshot["twcc"]["coverageRatio"], 1.0);
    assert_eq!(snapshot["twcc"]["ledgerHitRatio"], 0.0);
}

#[test]
fn build_observability_snapshot_projects_owner_contract_from_stats() {
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "recovery_owner_state": "rebuildingSupply",
        "recovery_owner_reason": "timelineReferenceBroken",
        "video_owner_source": "anchor",
        "video_owner_observed_at_ms": 2048.0,
        "video_health": "recovering",
        "primary_issue_chain": "recovery:timelineReferenceBroken",
        "latest_decision_summary": "owner:rebuildingSupply:timelineReferenceBroken"
    }));

    let snapshot = build_observability_snapshot(&stats);
    assert_eq!(snapshot["videoOwner"]["state"], "rebuildingSupply");
    assert_eq!(snapshot["videoOwner"]["reason"], "timelineReferenceBroken");
    assert_eq!(snapshot["videoOwner"]["source"], "anchor");
    assert_eq!(snapshot["videoOwner"]["observedAtMs"], 2048.0);
}

#[test]
fn record_runtime_trace_observations_uses_twcc_event_name_by_source() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut local_state = RuntimeTraceObservationState::default();
    let local_stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "twcc_observation_state": "local-feedback",
        "latest_video_twcc_observation": {
            "observation_id": 7,
            "source": "local-feedback",
            "feedback_packet_count": 3,
            "covered_sequence_start": 100,
            "covered_sequence_end": 220,
            "covered_sequence_span": 120,
            "observed_packet_count": 120,
            "observed_byte_count": 340000,
            "coverage_ratio": 1.0,
            "ledger_hit_ratio": 0.95,
            "feedback_interval_ms": 80.0,
            "arrival_span_ms": 70.0,
            "receive_bitrate_kbps": 22800.0,
            "twcc_sample_valid": true,
            "twcc_invalid_reason": null,
            "quality": "stable",
            "delivery_ratio": 0.99,
            "packet_loss_ratio": 0.01,
            "observed_at_ms": 2.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut local_state, Some("session-1"), &local_stats);
    let mut remote_state = RuntimeTraceObservationState::default();
    record_runtime_trace_observations(
        &recorder,
        &mut remote_state,
        Some("session-1"),
        &test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "twcc_observation_state": "remote-observed",
            "latest_video_twcc_observation": {
                "observation_id": 8,
                "source": "remote-rtcp",
                "feedback_packet_count": 3,
                "covered_sequence_start": 100,
                "covered_sequence_end": 220,
                "covered_sequence_span": 120,
                "observed_packet_count": 120,
                "observed_byte_count": 340000,
                "coverage_ratio": 1.0,
                "ledger_hit_ratio": null,
                "feedback_interval_ms": 80.0,
                "arrival_span_ms": 70.0,
                "receive_bitrate_kbps": 22800.0,
                "twcc_sample_valid": false,
                "twcc_invalid_reason": "source-remote",
                "quality": "remote-observed",
                "delivery_ratio": 0.99,
                "packet_loss_ratio": 0.01,
                "observed_at_ms": 2.0
            }
        })),
    );

    let contents = read_trace_lines(recorder.as_ref())
        .into_iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(contents.contains("\"event\":\"twccFeedbackSent\""));
    assert!(contents.contains("\"event\":\"twccFeedbackObserved\""));
}

#[test]
fn record_runtime_trace_observations_projects_remote_answer_acceptance() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_remote_answer_observation": {
            "observation_id": 11,
            "video_payload_order": [124, 97, 125],
            "selected_video_payload_type": 124,
            "selected_video_mime_type": "video/h264",
            "selected_video_profile_level_id": "4d002a",
            "accepted_video_rtcp_feedback": ["goog-remb", "transport-cc", "nack:pli"],
            "accepted_audio_rtcp_feedback": ["transport-cc"],
            "accepted_video_header_extensions": ["http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01#3"],
            "accepted_audio_header_extensions": ["urn:ietf:params:rtp-hdrext:ssrc-audio-level#2"],
            "observed_at_ms": 1234.0
        }
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let contents = read_trace_lines(recorder.as_ref())
        .into_iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(contents.contains("\"event\":\"remoteAnswerAccepted\""));
    assert!(contents.contains("\"selectedVideoProfileLevelId\":\"4d002a\""));
    assert!(contents.contains("\"googRembAccepted\":true"));
    assert!(contents.contains("\"transportCcAccepted\":true"));
}

#[test]
fn record_runtime_trace_observations_projects_keyframe_episode_lifecycle() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let make_stats = |episode_id,
                      status: &str,
                      request_kind: Option<&str>,
                      response_verdict: Option<&str>,
                      sent_at_ms: Option<f64>,
                      first_keyframe_packet_at_ms: Option<f64>,
                      first_keyframe_decoded_at_ms: Option<f64>,
                      response_rtp_timestamp: Option<u32>,
                      response_frame_seq: Option<u64>| {
        test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "latest_keyframe_request_episode": {
                "episode_id": episode_id,
                "request_reason": "transportAwaitRecoveryAnchor",
                "request_kind": request_kind,
                "status": status,
                "requested_at_ms": 1200.0,
                "sent_at_ms": sent_at_ms,
                "deadline_at_ms": 2160.0,
                "first_keyframe_packet_at_ms": first_keyframe_packet_at_ms,
                "first_keyframe_decoded_at_ms": first_keyframe_decoded_at_ms,
                "response_rtp_timestamp": response_rtp_timestamp,
                "response_frame_seq": response_frame_seq,
                "response_verdict": response_verdict
            }
        }))
    };

    record_runtime_trace_observations(
        &recorder,
        &mut state,
        Some("session-1"),
        &make_stats(
            91,
            "requested",
            None,
            Some("pending"),
            None,
            None,
            None,
            None,
            None,
        ),
    );
    record_runtime_trace_observations(
        &recorder,
        &mut state,
        Some("session-1"),
        &make_stats(
            91,
            "sent",
            Some("pli"),
            Some("pending"),
            Some(1210.0),
            None,
            None,
            None,
            None,
        ),
    );
    record_runtime_trace_observations(
        &recorder,
        &mut state,
        Some("session-1"),
        &make_stats(
            91,
            "packet-seen",
            Some("pli"),
            Some("on-time"),
            Some(1210.0),
            Some(1400.0),
            None,
            Some(123456789),
            None,
        ),
    );
    record_runtime_trace_observations(
        &recorder,
        &mut state,
        Some("session-1"),
        &make_stats(
            91,
            "decoded",
            Some("pli"),
            Some("on-time"),
            Some(1210.0),
            Some(1400.0),
            Some(1420.0),
            Some(123456789),
            Some(77),
        ),
    );
    record_runtime_trace_observations(
        &recorder,
        &mut state,
        Some("session-1"),
        &make_stats(
            92,
            "missed",
            Some("control"),
            Some("missed"),
            Some(1500.0),
            None,
            None,
            None,
            None,
        ),
    );

    let contents = read_trace_lines(recorder.as_ref())
        .into_iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(contents.contains("\"event\":\"keyframeRequestEpisode\""));
    assert!(contents.contains("\"status\":\"requested\""));
    assert!(contents.contains("\"event\":\"keyframeRequestEpisodeRequested\""));
    assert!(contents.contains("\"event\":\"keyframeRequestEpisodeSent\""));
    assert!(contents.contains("\"event\":\"keyframeRequestEpisodePacketSeen\""));
    assert!(contents.contains("\"event\":\"keyframeRequestEpisodeDecoded\""));
    assert!(contents.contains("\"event\":\"keyframeRequestEpisodeMissed\""));
    assert!(contents.contains("\"responseVerdict\":\"missed\""));
    assert!(contents.contains("\"requestToFirstPacketMs\":200.0"));
    assert!(contents.contains("\"requestToFirstDecodeMs\":220.0"));
    assert!(contents.contains("\"timedOut\":true"));
}

#[test]
fn record_runtime_trace_observations_emits_keyframe_succeeded_with_lifecycle_phase_in_payload() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 100,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "succeeded",
            "lifecycle_phase": "success",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "response_verdict": "cleanAnchorCommitted",
            "retired_at_ms": 1800.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "keyframeRequestEpisodeSucceeded");
    assert_eq!(payload["episodeId"], 100);
    assert_eq!(payload["lifecyclePhase"], "success");
    assert_eq!(payload["responseVerdict"], "cleanAnchorCommitted");
}

#[test]
fn record_runtime_trace_observations_projects_keyframe_response_observed_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 93,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "response-observed",
            "status_detail": "bootstrapMissingSps",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "first_video_packet_at_ms": 1300.0,
            "first_video_packet_rtp_timestamp": 123456000,
            "first_video_packet_is_keyframe": true,
            "first_keyframe_packet_at_ms": 1300.0,
            "response_rtp_timestamp": 123456000,
            "response_verdict": "pending"
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "keyframeRequestEpisodeResponseObserved");
    assert_eq!(payload["episodeId"], 93);
    assert_eq!(payload["status"], "response-observed");
    assert_eq!(payload["statusDetail"], "bootstrapMissingSps");
    assert_eq!(payload["firstVideoPacketIsKeyframe"], true);
}

#[test]
fn record_runtime_trace_observations_projects_keyframe_transport_suppression_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_timeline_observation": {
            "observation_id": 301,
            "source_event": "frame-inspection-rejected-await-anchor",
            "gap": null,
            "frame": {
                "state": "waiting-keyframe",
                "frame_rtp_timestamp": 123456001,
                "is_keyframe": false,
                "frame_importance": "reference",
                "close_reason": "bootstrapMissingSps",
                "observed_at_ms": 1290.0
            },
            "chain": {
                "state": "waiting-keyframe",
                "reason": "transportAwaitRecoveryAnchor",
                "observed_at_ms": 1290.0
            },
            "observed_at_ms": 1290.0
        },
        "latest_keyframe_request_episode": {
            "episode_id": 94,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "deferred",
            "status_detail": "transport-suppressed",
            "requested_at_ms": 1200.0,
            "deadline_at_ms": 2160.0,
            "transport_detail": "coalesced:keyframeInFlight",
            "response_verdict": "pending",
            "family_id": "transportAwaitRecoveryAnchor:pli",
            "owner_episode_id": 90,
            "suppress_duration_ms": 180.0,
            "release_reason": "ownerEpisodeSucceeded"
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "keyframeRequestSuppressedObserved");
    assert_eq!(payload["episodeId"], 94);
    assert_eq!(payload["status"], "deferred");
    assert_eq!(payload["statusDetail"], "transport-suppressed");
    assert_eq!(payload["transportDetail"], "coalesced:keyframeInFlight");
    assert_eq!(payload["familyId"], "transportAwaitRecoveryAnchor:pli");
    assert_eq!(payload["ownerEpisodeId"], 90);
    assert_eq!(payload["suppressDurationMs"], 180.0);
    assert_eq!(payload["releaseReason"], "ownerEpisodeSucceeded");
    assert_eq!(
        payload["diagnosticTimelineSourceEvent"],
        "frame-inspection-rejected-await-anchor"
    );
    assert_eq!(
        payload["diagnosticSuppressionReason"],
        "coalesced:keyframeInFlight"
    );
}

#[test]
fn record_runtime_trace_observations_projects_h264_inspection_rejection() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_h264_inspection_observation": {
            "observation_id": 2194672423u64,
            "frame_rtp_timestamp": 2194672423u32,
            "nal_types": ["SliceLayerWithoutPartitioningIdr"],
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": false,
            "committed_pps_present": false,
            "slice_headers_valid": false,
            "delta_continuation_ready": false,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": true,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "bootstrapMissingSps",
            "admission_accepted": false,
            "observed_at_ms": 1400.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let state_payload = find_event_payload(&entries, "h264Inspection");
    assert_eq!(
        state_payload["bootstrapRejectReason"],
        "bootstrapMissingSps"
    );
    assert_eq!(state_payload["committedSpsPresent"], false);
    assert_eq!(state_payload["hasInbandSps"], false);
    assert_eq!(state_payload["hasInbandPps"], false);
    assert_eq!(state_payload["admissionAccepted"], false);
    assert_eq!(state_payload["linkedEpisodeId"], Value::Null);
    let payload = find_event_payload(&entries, "h264InspectionRejected");
    assert_eq!(payload["observationId"], 2194672423u64);
    assert_eq!(payload["frameRtpTimestamp"], 2194672423u32);
    assert_eq!(payload["bootstrapRejectReason"], "bootstrapMissingSps");
    assert_eq!(payload["committedSpsPresent"], false);
    assert_eq!(payload["committedPpsPresent"], false);
}

#[test]
fn record_runtime_trace_observations_projects_bootstrap_reject_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 95,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "packet-seen",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "first_keyframe_packet_at_ms": 1399.0,
            "response_rtp_timestamp": 2194672445u32,
            "response_verdict": "pending"
        },
        "latest_h264_inspection_observation": {
            "observation_id": 2194672445u64,
            "frame_rtp_timestamp": 2194672445u32,
            "nal_types": ["SliceLayerWithoutPartitioningNonIdr"],
            "nal_count": 1,
            "vcl_nal_count": 1,
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": false,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "NonIdrVcl",
            "admission_accepted": false,
            "observed_at_ms": 1400.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "bootstrapRejectObserved");
    assert_eq!(payload["frameRtpTimestamp"], 2194672445u32);
    assert_eq!(payload["bootstrapRejectReason"], "NonIdrVcl");
    assert_eq!(payload["isIdr"], false);
    assert_eq!(payload["admissionAccepted"], false);
    assert_eq!(payload["linkedEpisodeId"], 95);
    assert_eq!(payload["isRecoveryKeyframeResponseContext"], true);
    assert_eq!(payload["usableIdrOutcome"], "missingUsableIdr");
}

#[test]
fn bootstrap_reject_observed_classifies_non_idr_before_usable_idr() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 96,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "decoded",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "first_video_packet_at_ms": 1300.0,
            "first_video_packet_rtp_timestamp": 22334455,
            "first_video_packet_is_keyframe": false,
            "first_keyframe_decoded_at_ms": 1450.0,
            "response_rtp_timestamp": 22334455,
            "response_verdict": "on-time"
        },
        "latest_h264_inspection_observation": {
            "observation_id": 22334455u64,
            "frame_rtp_timestamp": 22334455u32,
            "nal_types": ["SliceLayerWithoutPartitioningNonIdr"],
            "nal_count": 1,
            "vcl_nal_count": 1,
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": true,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "NonIdrVcl",
            "admission_accepted": false,
            "observed_at_ms": 1300.0,
            "bound_episode_id": 96,
            "bound_as_recovery_response": true
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "bootstrapRejectObserved");
    assert_eq!(payload["usableIdrOutcome"], "beforeUsableIdr");
}

#[test]
fn bootstrap_reject_observed_classifies_non_idr_without_usable_idr() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 97,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "missed",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "first_video_packet_at_ms": 1300.0,
            "first_video_packet_rtp_timestamp": 22334456,
            "first_video_packet_is_keyframe": false,
            "response_rtp_timestamp": 22334456,
            "response_verdict": "missed",
            "retired_at_ms": 2160.0
        },
        "latest_h264_inspection_observation": {
            "observation_id": 22334456u64,
            "frame_rtp_timestamp": 22334456u32,
            "nal_types": ["SliceLayerWithoutPartitioningNonIdr"],
            "nal_count": 1,
            "vcl_nal_count": 1,
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": true,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "NonIdrVcl",
            "admission_accepted": false,
            "observed_at_ms": 1300.0,
            "bound_episode_id": 97,
            "bound_as_recovery_response": true
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "bootstrapRejectObserved");
    assert_eq!(payload["usableIdrOutcome"], "missingUsableIdr");
}

#[test]
fn record_runtime_trace_observations_keeps_bootstrap_gap_delta_slice_as_observed() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_h264_inspection_observation": {
            "observation_id": 2194672444u64,
            "frame_rtp_timestamp": 2194672444u32,
            "nal_types": ["SliceLayerWithoutPartitioningNonIdr"],
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": true,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "bootstrapMissingSps",
            "admission_accepted": true,
            "observed_at_ms": 1400.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let state_payload = find_event_payload(&entries, "h264Inspection");
    assert_eq!(
        state_payload["bootstrapRejectReason"],
        "bootstrapMissingSps"
    );
    assert_eq!(state_payload["deltaContinuationReady"], true);
    assert_eq!(state_payload["admissionAccepted"], true);
    let payload = find_event_payload(&entries, "h264InspectionObserved");
    assert_eq!(payload["observationId"], 2194672444u64);
    assert_eq!(payload["bootstrapRejectReason"], "bootstrapMissingSps");
    assert_eq!(payload["admissionAccepted"], true);
}

#[test]
fn h264_inspection_observed_projects_post_recovery_degradation_flag() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_h264_inspection_observation": {
            "observation_id": 7,
            "frame_rtp_timestamp": 888,
            "nal_types": [],
            "nal_count": 0,
            "vcl_nal_count": 0,
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": true,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "bootstrapMissingIdr",
            "continuation_verdict": "continuationAcceptedWhileAwaitingIdr",
            "admission_accepted": true,
            "observed_at_ms": 200.0,
            "bound_episode_id": 43,
            "bound_episode_status": "decoded",
            "bound_as_recovery_response": true,
            "bound_response_rtp_timestamp": 888,
            "bound_recovery_epoch": 1,
            "episode_phase_at_observation": "decoded",
            "is_post_recovery_degradation": true,
            "reject_classification": "continuationAcceptedWhileAwaitingIdr"
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "h264InspectionObserved");
    assert_eq!(payload["isPostRecoveryDegradation"], true);
    assert_eq!(
        payload["rejectClassification"],
        "continuationAcceptedWhileAwaitingIdr"
    );
}

#[test]
fn record_runtime_trace_observations_emits_recovery_collection_events() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_picture_recovery_transition_observation": {
            "observation_id": 11,
            "episode_id": 7,
            "recovery_epoch": 3,
            "phase": "CleanAnchorCommitted",
            "from_phase": "Decoded",
            "to_phase": "CleanAnchorCommitted",
            "cause": "chain-clean-anchor-submitted",
            "detail": "mediaGate",
            "rtp_timestamp": 123456,
            "frame_seq": 42,
            "owner_state": "stable-serving",
            "transport_state": "Connected",
            "observed_at_ms": 180.0
        },
        "latest_picture_recovery_blocker_observation": {
            "observation_id": 12,
            "episode_id": 7,
            "recovery_epoch": 3,
            "gate": "media",
            "blocker_kind": "localWindowAcceptedButBootstrapRejected",
            "severity": "warning",
            "first_observed_at_ms": 181.0,
            "observed_at_ms": 182.0,
            "count": 2,
            "frame_rtp_timestamp": 123460,
            "frame_seq": 43,
            "owner_state": "supply-starved",
            "transport_state": "Connected"
        },
        "latest_video_ingress_termination_observation": {
            "observation_id": 13,
            "termination_id": 5,
            "derived_from_termination_id": 5,
            "kind": "rxClosed",
            "cause": "upstreamSenderDropped",
            "upstream_cause": "trackEnded",
            "source_subsystem": "video-ingress",
            "linked_recovery_epoch": 3,
            "linked_episode_id": 7,
            "transport_state": "Connected",
            "owner_state": "supply-starved",
            "video_track_state": "remoteTrackAttached",
            "recent_command": "rtcVideoIngressRxClosed",
            "observed_at_ms": 183.0
        },
        "latest_first_frame_latency_observation": {
            "observation_id": 14,
            "episode_id": 7,
            "recovery_epoch": 3,
            "control_ready_to_pli_sent_ms": 20.0,
            "pli_sent_to_first_idr_packet_ms": 100.0,
            "first_idr_packet_to_first_decode_ms": 260.0,
            "first_decode_to_clean_anchor_committed_ms": 12.0,
            "clean_anchor_committed_to_display_stable_ms": 28.0,
            "terminal_phase": "DisplayStable",
            "incomplete_reason": null,
            "observed_at_ms": 184.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    assert_eq!(
        find_event_payload(&entries, "pictureRecoveryTransition")["toPhase"],
        "CleanAnchorCommitted"
    );
    assert_eq!(
        find_event_payload(&entries, "pictureRecoveryBlockerObserved")["blockerKind"],
        "localWindowAcceptedButBootstrapRejected"
    );
    assert_eq!(
        find_event_payload(&entries, "videoIngressTermination")["kind"],
        "rxClosed"
    );
    assert_eq!(
        find_event_payload(&entries, "firstFrameLatencyObserved")["terminalPhase"],
        "DisplayStable"
    );
}

#[test]
fn record_runtime_trace_observations_correlates_keyframe_and_h264_context() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 91,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "packet-seen",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "first_keyframe_packet_at_ms": 1400.0,
            "response_rtp_timestamp": 2194672423u32,
            "response_verdict": "on-time"
        },
        "latest_h264_inspection_observation": {
            "observation_id": 2194672423u64,
            "frame_rtp_timestamp": 2194672423u32,
            "nal_types": ["SliceLayerWithoutPartitioningNonIdr"],
            "nal_count": 1,
            "vcl_nal_count": 1,
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": false,
            "committed_pps_present": false,
            "slice_headers_valid": false,
            "delta_continuation_ready": false,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "sample_width": 1280,
            "sample_height": 720,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "bootstrapMissingSps",
            "admission_accepted": false,
            "observed_at_ms": 1401.0
        },
        "latest_video_rtcp_send_failure_time_ms": 1300.0,
        "latest_video_rtcp_send_failure_reason": "rtcp-write-failed"
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let episode_payload = find_event_payload(&entries, "keyframeRequestEpisode");
    assert_eq!(
        episode_payload["linkedH264BootstrapRejectReason"],
        "bootstrapMissingSps"
    );
    assert_eq!(episode_payload["linkedH264AdmissionAccepted"], false);
    assert_eq!(
        episode_payload["recentRtcpSendFailureReason"],
        "rtcp-write-failed"
    );

    let inspection_payload = find_event_payload(&entries, "h264Inspection");
    assert_eq!(inspection_payload["linkedEpisodeId"], 91);
    assert_eq!(inspection_payload["linkedEpisodeStatus"], "packet-seen");
    assert_eq!(
        inspection_payload["linkedEpisodeRequestReason"],
        "transportAwaitRecoveryAnchor"
    );
    assert_eq!(
        inspection_payload["isRecoveryKeyframeResponseContext"],
        true
    );

    let rtcp_failure_payload = find_event_payload(&entries, "videoRtcpSendFailureObserved");
    assert_eq!(rtcp_failure_payload["reason"], "rtcp-write-failed");
}

#[test]
fn record_runtime_trace_observations_projects_keyframe_episode_recovery_diagnostics() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 101,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "response-observed",
            "requested_at_ms": 1200.0,
            "sent_at_ms": 1210.0,
            "deadline_at_ms": 2160.0,
            "first_video_packet_at_ms": 1300.0,
            "first_video_packet_rtp_timestamp": 22334455,
            "first_video_packet_is_keyframe": false,
            "response_verdict": "pending"
        },
        "latest_h264_inspection_observation": {
            "observation_id": 22334455u64,
            "frame_rtp_timestamp": 22334455u32,
            "nal_types": ["SliceLayerWithoutPartitioningNonIdr"],
            "nal_count": 1,
            "vcl_nal_count": 1,
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": true,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "sample_width": 1920,
            "sample_height": 1080,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "NonIdrVcl",
            "admission_accepted": true,
            "observed_at_ms": 1300.0
        },
        "latest_video_timeline_observation": {
            "observation_id": 77,
            "source_event": "frame-inspection-rejected-await-anchor",
            "gap": {
                "state": "expired",
                "sequence": 28519,
                "frame_rtp_timestamp": 22334455,
                "frame_importance": "anchor",
                "budget_importance": "anchor",
                "evidence_importance": "anchor",
                "gap_dependency_confidence": "bound",
                "observed_at_ms": 1300.0
            },
            "frame": {
                "state": "closed",
                "frame_rtp_timestamp": 22334455,
                "is_keyframe": false,
                "frame_importance": "unknown",
                "budget_importance": "unknown",
                "evidence_importance": "unknown",
                "close_reason": "inspectionRejectNonIdrVcl",
                "observed_at_ms": 1300.0
            },
            "chain": {
                "state": "recovering",
                "reason": "awaitingRecoveryAnchor",
                "observed_at_ms": 1300.0
            },
            "observed_at_ms": 1300.0
        },
        "latest_anchor_candidate_ledger": {
            "recovery_epoch": 9,
            "frame_rtp_timestamp": 22334455,
            "state": "rejected",
            "source_event": "frame-inspection-rejected-await-anchor",
            "failure_reason": "unknown",
            "observed_at_ms": 1300.0
        },
        "latest_decode_candidate_decision": {
            "decision_id": 5,
            "state": "blocked",
            "action": "drop",
            "detail": "outputQueueOverflow",
            "frame_seq": 12,
            "observed_at_ms": 1301.0
        },
        "latest_decode_output_path_observation": {
            "observation_id": 6,
            "verdict": "decoded-frame",
            "detail": "decodedFrameReady",
            "frame_rtp_timestamp": 22334455,
            "is_keyframe": true,
            "send_packet_status": 0,
            "receive_frame_status": -35,
            "backend_no_output_streak": 0,
            "input_frames_since_last_decoded": 1,
            "bootstrap_reject_reason": null,
            "observed_at_ms": 1302.0
        },
        "latest_video_frame_drop": {
            "observation_id": 7,
            "reason": "decode:drop:outputQueueOverflow",
            "stage": "decode",
            "action": "drop",
            "detail": "outputQueueOverflow",
            "frame_rtp_timestamp": 22334455,
            "frame_seq": 12,
            "frame_recovery_disposition": "repairing",
            "observed_at_ms": 1303.0,
            "width": 1920,
            "height": 1080,
            "is_keyframe": false,
            "queue_depth": 3
        },
        "latest_recovery_decision_ledger": {
            "decision_id": 9001,
            "state_before": "observing",
            "state_after": "recovery-blocked",
            "input_signal": "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor",
            "gate_result": "coalesced:keyframeInFlight",
            "action_selected": "coalesced:keyframeInFlight",
            "recovery_episode_stage": "bootstrap",
            "budget_after": {
                "recovery_epoch": 9,
                "keyframe_budget_used": 1,
                "keyframe_budget_limit": 255,
                "decoder_reset_budget_used": 0,
                "decoder_reset_budget_limit": 255,
                "reconnect_budget_used": 0,
                "reconnect_budget_limit": 1
            },
            "observed_at_ms": 1304.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "keyframeRequestEpisode");
    assert_eq!(
        payload["diagnosticPendingReason"],
        "anchorRejected:h264Reject:NonIdrVcl"
    );
    assert_eq!(payload["diagnosticRecoveryEpoch"], 9);
    assert_eq!(
        payload["diagnosticTimelineSourceEvent"],
        "frame-inspection-rejected-await-anchor"
    );
    assert_eq!(payload["diagnosticTimelineChainState"], "recovering");
    assert_eq!(
        payload["diagnosticTimelineChainReason"],
        "awaitingRecoveryAnchor"
    );
    assert_eq!(payload["diagnosticAnchorState"], "rejected");
    assert_eq!(payload["diagnosticAnchorFailureReason"], "unknown");
    assert_eq!(
        payload["diagnosticAnchorSourceEvent"],
        "frame-inspection-rejected-await-anchor"
    );
    assert_eq!(payload["diagnosticDecodeCandidateAction"], "drop");
    assert_eq!(
        payload["diagnosticDecodeCandidateDetail"],
        "outputQueueOverflow"
    );
    assert_eq!(payload["diagnosticFrameDropDetail"], "outputQueueOverflow");
    assert_eq!(payload["diagnosticFrameDropQueueDepth"], 3);
    assert_eq!(payload["diagnosticDecodeOutputDetail"], "decodedFrameReady");
}

#[test]
fn bwe_updated_event_uses_top_level_actual_video_bitrate() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "199.7ms",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "0.7Mbps",
        "decode": "",
        "actual_video_bitrate_source": "transport-metrics",
        "video_actual_bitrate_kbps": 1019.4,
        "latest_video_bwe_observation": {
            "observation_id": 42,
            "mode": "twcc-gcc",
            "decision_reason": "twcc-gcc-cloud-ramp-up",
            "target_remb_kbps": 28500,
            "observed_remb_kbps": 28500,
            "actual_video_bitrate_kbps": 0.0,
            "loss_ratio": 0.0,
            "rtt_ms": 199.7,
            "transport_path": "Direct",
            "twcc_feedback_interval_ms": 113.0,
            "twcc_observed_packet_count": 12,
            "twcc_covered_sequence_span": 12,
            "twcc_receive_bitrate_kbps": 1019.4,
            "twcc_delivery_ratio": 1.0,
            "twcc_loss_ratio": 0.0,
            "observed_at_ms": 1000.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let contents = read_trace_lines(recorder.as_ref())
        .into_iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(contents.contains("\"event\":\"bweUpdated\""));
    assert!(contents.contains("\"actualVideoBitrateKbps\":1019.4"));
    assert!(!contents.contains("\"actualVideoBitrateKbps\":0.0"));
}

#[test]
fn frame_recovery_observation_projects_ledger_events() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_frame_recovery_observation": {
            "observation_id": 77,
            "action": "ledgerWrite",
            "frame_rtp_timestamp": 123456789,
            "frame_playout_deadline_at_ms": 4567.0,
            "frame_recovery_disposition": "unrecoverable-reference-chain",
            "frame_unrecoverable_reason": "referenceChainUnrecoverable",
            "observed_at_ms": 1234.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let contents = read_trace_lines(recorder.as_ref())
        .into_iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(contents.contains("\"event\":\"frameRecoveryObserved\""));
    assert!(contents.contains("\"action\":\"ledgerWrite\""));
    assert!(contents.contains("\"frameRtpTimestamp\":123456789"));
    assert!(contents.contains("\"frameRecoveryDisposition\":\"unrecoverable-reference-chain\""));
    assert!(contents.contains("\"frameUnrecoverableReason\":\"referenceChainUnrecoverable\""));
}

#[test]
fn steady_frame_recovery_observation_projects_null_disposition() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_frame_recovery_observation": {
            "observation_id": 78,
            "action": "ledgerWrite",
            "frame_rtp_timestamp": 123456790,
            "frame_playout_deadline_at_ms": null,
            "frame_recovery_disposition": null,
            "frame_unrecoverable_reason": null,
            "observed_at_ms": 1235.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let contents = read_trace_lines(recorder.as_ref())
        .into_iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(contents.contains("\"event\":\"frameRecoveryObserved\""));
    assert!(contents.contains("\"frameRecoveryDisposition\":null"));
}

#[test]
fn video_timeline_observation_projects_event_and_snapshot() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_timeline_observation": {
            "observation_id": 88,
            "source_event": "gap-repair-in-flight",
            "gap": {
                "state": "repair-in-flight",
                "sequence": 1337,
                "frame_rtp_timestamp": 123456789,
                "frame_importance": "reference",
                "observed_at_ms": 1250.0
            },
            "frame": {
                "state": "repairing",
                "frame_rtp_timestamp": 123456789,
                "is_keyframe": false,
                "frame_importance": "reference",
                "close_reason": null,
                "observed_at_ms": 1250.0
            },
            "chain": {
                "state": "repairing",
                "reason": "gapRepairInFlight",
                "observed_at_ms": 1250.0
            },
            "observed_at_ms": 1250.0
        }
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "videoTimelineObserved");
    assert_eq!(payload["sourceEvent"], "gap-repair-in-flight");
    assert_eq!(payload["gap"]["state"], "repair-in-flight");
    assert_eq!(payload["frame"]["state"], "repairing");
    assert_eq!(payload["chain"]["state"], "repairing");
    assert!(entries
        .iter()
        .all(|entry| entry["event"] != "videoTimeoutTransition"));
}

#[test]
fn anchor_candidate_ledger_projects_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_anchor_candidate_ledger": {
            "recovery_epoch": 9,
            "frame_rtp_timestamp": 123456799,
            "state": "rejected",
            "source_event": "frame-inspection-rejected-await-anchor",
            "failure_reason": "inspectionRejectInvalidSliceHeader",
            "observed_at_ms": 2250.0
        }
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "videoAnchorCandidateObserved");
    assert_eq!(payload["recoveryEpoch"], 9);
    assert_eq!(payload["frameRtpTimestamp"], 123456799);
    assert_eq!(payload["state"], "rejected");
    assert_eq!(
        payload["sourceEvent"],
        "frame-inspection-rejected-await-anchor"
    );
    assert_eq!(
        payload["failureReason"],
        "inspectionRejectInvalidSliceHeader"
    );
}

#[test]
fn timeout_video_timeline_observation_projects_timeout_transition_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_timeline_observation": {
            "observation_id": 89,
            "source_event": "timeout-stream-idle",
            "gap": null,
            "frame": {
                "state": "waiting-keyframe",
                "frame_rtp_timestamp": 123456790,
                "is_keyframe": false,
                "frame_importance": "reference",
                "close_reason": null,
                "observed_at_ms": 1300.0
            },
            "chain": {
                "state": "stalled",
                "reason": "streamIdleTimeout",
                "observed_at_ms": 1300.0
            },
            "observed_at_ms": 1300.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "videoTimeoutTransition");
    assert_eq!(payload["observationId"], 89);
    assert_eq!(payload["sourceEvent"], "timeout-stream-idle");
    assert_eq!(payload["chain"]["state"], "stalled");
    assert_eq!(payload["chain"]["reason"], "streamIdleTimeout");
    assert_eq!(payload["observedAtMs"], 1300.0);
    assert!(entries
        .iter()
        .any(|entry| entry["event"] == "videoTimelineObserved"));
}

#[test]
fn chain_broken_timeline_projects_chain_transition_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_timeline_observation": {
            "observation_id": 90,
            "source_event": "chain-broken",
            "gap": null,
            "frame": null,
            "chain": {
                "state": "broken",
                "reason": "referenceChainBroken",
                "observed_at_ms": 1400.0
            },
            "observed_at_ms": 1400.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "videoChainTransition");
    assert_eq!(payload["observationId"], 90);
    assert_eq!(payload["sourceEvent"], "chain-broken");
    assert_eq!(payload["previousChainState"], serde_json::Value::Null);
    assert_eq!(payload["state"], "broken");
    assert_eq!(payload["reason"], "referenceChainBroken");
    assert_eq!(payload["chain"]["state"], "broken");
    assert_eq!(payload["chain"]["reason"], "referenceChainBroken");
}

#[test]
fn clean_anchor_funnel_projects_ingress_blocked_and_submitted_events() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();

    let ingress_stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_anchor_candidate_ledger": {
            "recovery_epoch": 12,
            "frame_rtp_timestamp": 4001,
            "state": "observed",
            "source_event": "frame-complete-candidate",
            "failure_reason": null,
            "observed_at_ms": 1500.0
        },
        "latest_video_timeline_observation": {
            "observation_id": 401,
            "source_event": "frame-complete-candidate",
            "gap": null,
            "frame": {
                "state": "complete-candidate",
                "frame_rtp_timestamp": 4001,
                "is_keyframe": true,
                "frame_importance": "anchor",
                "close_reason": null,
                "observed_at_ms": 1500.0
            },
            "chain": {
                "state": "recovering",
                "reason": "awaitingCleanAnchor",
                "observed_at_ms": 1500.0
            },
            "observed_at_ms": 1500.0
        }
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &ingress_stats);

    let blocked_stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_anchor_candidate_ledger": {
            "recovery_epoch": 12,
            "frame_rtp_timestamp": 4002,
            "state": "observed",
            "source_event": "frame-complete-candidate-decode-feedback-blocked",
            "failure_reason": null,
            "observed_at_ms": 1510.0
        },
        "latest_video_timeline_observation": {
            "observation_id": 402,
            "source_event": "frame-complete-candidate-decode-feedback-blocked",
            "gap": null,
            "frame": {
                "state": "complete-candidate",
                "frame_rtp_timestamp": 4002,
                "is_keyframe": true,
                "frame_importance": "anchor",
                "close_reason": null,
                "observed_at_ms": 1510.0
            },
            "chain": {
                "state": "recovering",
                "reason": "decodeFeedbackBlocked",
                "observed_at_ms": 1510.0
            },
            "observed_at_ms": 1510.0
        }
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &blocked_stats);

    let submitted_stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_anchor_candidate_ledger": {
            "recovery_epoch": 12,
            "frame_rtp_timestamp": 4001,
            "state": "submitted-clean-anchor",
            "source_event": "chain-clean-anchor-submitted",
            "failure_reason": null,
            "observed_at_ms": 1520.0
        },
        "latest_video_timeline_observation": {
            "observation_id": 403,
            "source_event": "chain-clean-anchor-submitted",
            "gap": null,
            "frame": {
                "state": "complete-candidate",
                "frame_rtp_timestamp": 4001,
                "is_keyframe": true,
                "frame_importance": "anchor",
                "close_reason": null,
                "observed_at_ms": 1520.0
            },
            "chain": {
                "state": "steady",
                "reason": "cleanAnchorCommitted",
                "observed_at_ms": 1520.0
            },
            "observed_at_ms": 1520.0
        }
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &submitted_stats);

    let entries = read_trace_lines(recorder.as_ref());

    let candidate_payload = find_event_payload(&entries, "cleanAnchorCompleteCandidateObserved");
    assert_eq!(candidate_payload["frameRtpTimestamp"], 4001);
    assert_eq!(candidate_payload["chainState"], "recovering");
    assert_eq!(candidate_payload["recoveryEpoch"], 12);

    let blocked_payload = find_event_payload(&entries, "cleanAnchorCompleteCandidateBlocked");
    assert_eq!(blocked_payload["frameRtpTimestamp"], 4002);
    assert_eq!(blocked_payload["chainReason"], "decodeFeedbackBlocked");
    assert_eq!(
        blocked_payload["sourceEvent"],
        "frame-complete-candidate-decode-feedback-blocked"
    );

    let submitted_payload = find_event_payload(&entries, "cleanAnchorSubmitted");
    assert_eq!(submitted_payload["frameRtpTimestamp"], 4001);
    assert_eq!(submitted_payload["chainState"], "steady");
    assert_eq!(submitted_payload["anchorState"], "submitted-clean-anchor");
}

#[test]
fn chain_flush_timeline_projects_backlog_flushed_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_timeline_observation": {
            "observation_id": 91,
            "source_event": "gap-expired-chain-flush",
            "gap": {
                "state": "expired",
                "sequence": 2001,
                "frame_rtp_timestamp": 22334455,
                "frame_importance": "reference",
                "observed_at_ms": 1410.0
            },
            "frame": {
                "state": "dropped",
                "frame_rtp_timestamp": 22334455,
                "is_keyframe": false,
                "frame_importance": "reference",
                "close_reason": "chainFlush",
                "observed_at_ms": 1410.0
            },
            "chain": {
                "state": "recovering",
                "reason": "referenceChainBroken",
                "observed_at_ms": 1410.0
            },
            "observed_at_ms": 1410.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "videoBacklogFlushed");
    assert_eq!(payload["observationId"], 91);
    assert_eq!(payload["sourceEvent"], "gap-expired-chain-flush");
    assert_eq!(payload["gap"]["sequence"], 2001);
    assert_eq!(payload["chain"]["state"], "recovering");
}

#[test]
fn nack_observation_projects_event_name_and_common_fields() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();

    let cases = [
        ("skipped", "nackSkipped"),
        ("expiredDeadline", "nackExpired"),
        ("sent", "nackSent"),
    ];

    for (observation_id, (action, expected_event)) in (1_u64..).zip(cases.iter()) {
        let stats = test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "latest_video_nack_observation": {
                "observation_id": observation_id,
                "action": action,
                "source": "scheduler",
                "first_sequence": 100,
                "last_sequence": 102,
                "packet_count": 3,
                "retry_count": 1,
                "frame_rtp_timestamp": 123456789,
                "frame_is_keyframe": false,
                "frame_importance": "delta",
                "deadline_at_ms": 1500.0,
                "estimated_recovery_arrival_ms": 1512.0,
                "nack_disposition": "skippedTooLate",
                "frame_playout_deadline_at_ms": 1498.0,
                "frame_unrecoverable_reason": "referenceChainUnrecoverable",
                "observed_at_ms": 1200.0
            }
        }));
        record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

        let entries = read_trace_lines(recorder.as_ref());
        let payload = find_event_payload(&entries, expected_event);
        assert_eq!(payload["action"], *action);
        assert_eq!(payload["deadlineAtMs"], 1500.0);
        assert_eq!(payload["estimatedRecoveryArrivalMs"], 1512.0);
        assert_eq!(payload["nackDisposition"], "skippedTooLate");
        assert_eq!(payload["framePlayoutDeadlineAtMs"], 1498.0);
        assert_eq!(
            payload["frameUnrecoverableReason"],
            "referenceChainUnrecoverable"
        );
        assert_eq!(payload["frameImportance"], "delta");
    }
}

#[test]
fn stall_transition_does_not_repeat_when_values_unchanged() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "video_decoder_stalled": false,
        "video_renderer_stalled": true,
        "packet_age_ms": 10.0,
        "decode_age_ms": 20.0,
        "present_age_ms": 30.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

    let entries = read_trace_lines(recorder.as_ref());
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["event"] == "videoDecoderStallTransition")
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["event"] == "videoRendererStallTransition")
            .count(),
        1
    );
}

#[test]
fn decoder_stall_transition_emits_for_false_true_and_true_false() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();

    let stats_false = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "video_decoder_stalled": false,
        "video_renderer_stalled": false,
        "packet_age_ms": 10.0,
        "decode_age_ms": 20.0,
        "present_age_ms": 30.0
    }));
    let stats_true = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "video_decoder_stalled": true,
        "video_renderer_stalled": false,
        "packet_age_ms": 11.0,
        "decode_age_ms": 21.0,
        "present_age_ms": 31.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_false);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_true);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_false);

    let entries = read_trace_lines(recorder.as_ref());
    let decoder_payloads = event_payloads(&entries, "videoDecoderStallTransition");
    assert!(decoder_payloads
        .iter()
        .any(|payload| payload["previousStalled"] == false && payload["stalled"] == true));
    assert!(decoder_payloads
        .iter()
        .any(|payload| payload["previousStalled"] == true && payload["stalled"] == false));
}

#[test]
fn decoder_and_renderer_stall_transition_are_triggered_independently() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();

    let stats_base = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "video_decoder_stalled": false,
        "video_renderer_stalled": false
    }));
    let stats_decoder_only = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "video_decoder_stalled": true,
        "video_renderer_stalled": false
    }));
    let stats_renderer_only = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "video_decoder_stalled": true,
        "video_renderer_stalled": true
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_base);
    record_runtime_trace_observations(
        &recorder,
        &mut state,
        Some("session-1"),
        &stats_decoder_only,
    );
    record_runtime_trace_observations(
        &recorder,
        &mut state,
        Some("session-1"),
        &stats_renderer_only,
    );

    let entries = read_trace_lines(recorder.as_ref());
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["event"] == "videoDecoderStallTransition")
            .count(),
        2
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["event"] == "videoRendererStallTransition")
            .count(),
        2
    );

    let renderer_payloads = event_payloads(&entries, "videoRendererStallTransition");
    assert!(renderer_payloads
        .iter()
        .any(|payload| payload["previousStalled"] == false && payload["stalled"] == true));
}

#[test]
fn frame_drop_decode_stage_does_not_project_decode_candidate_decision() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_frame_drop": {
            "observation_id": 501,
            "stage": "decode",
            "action": "drop",
            "detail": "outputQueueOverflow",
            "reason": "dropBackpressure",
            "observed_at_ms": 1001.0,
            "width": 1920,
            "height": 1080,
            "is_keyframe": false,
            "queue_depth": 2,
            "frame_rtp_timestamp": 123456789,
            "frame_seq": 77,
            "frame_recovery_disposition": "repairing",
            "frame_unrecoverable_reason": null
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    assert!(has_event(&entries, "frameDropped"));
    assert!(!has_event(&entries, "decodeCandidateDecision"));
}

#[test]
fn frame_drop_render_stage_projects_render_candidate_decision() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_frame_drop": {
            "observation_id": 502,
            "stage": "render",
            "action": "replace",
            "detail": "mailboxOverwrite",
            "reason": "dropBackpressure",
            "observed_at_ms": 1002.0,
            "width": 1280,
            "height": 720,
            "is_keyframe": false,
            "queue_depth": 1,
            "frame_rtp_timestamp": 123456790,
            "frame_seq": 78,
            "frame_recovery_disposition": "repairing",
            "frame_unrecoverable_reason": null
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    assert!(entries.iter().any(|entry| entry["event"] == "frameDropped"));
    let payload = find_event_payload(&entries, "renderMailboxDecision");
    assert_eq!(payload["stage"], "render");
    assert_eq!(payload["detail"], "mailboxOverwrite");
    assert_eq!(payload["reason"], "dropBackpressure");
}

#[test]
fn frame_drop_unknown_stage_does_not_project_candidate_decision() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_frame_drop": {
            "observation_id": 503,
            "stage": "ingress",
            "action": "drop",
            "detail": "predecode",
            "reason": "dropBackpressure",
            "observed_at_ms": 1003.0,
            "width": 640,
            "height": 360,
            "is_keyframe": false,
            "queue_depth": 3,
            "frame_rtp_timestamp": 123456791,
            "frame_seq": 79,
            "frame_recovery_disposition": "repairing",
            "frame_unrecoverable_reason": null
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    assert!(entries.iter().any(|entry| entry["event"] == "frameDropped"));
    assert!(entries
        .iter()
        .all(|entry| entry["event"] != "decodeCandidateDecision"));
    assert!(entries
        .iter()
        .all(|entry| entry["event"] != "renderMailboxDecision"));
}

#[test]
fn decode_candidate_state_does_not_project_transition_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_decode_candidate_decision": {
            "decision_id": 701,
            "state": "backpressure",
            "action": "drop",
            "detail": "outputQueueOverflow",
            "frame_seq": 42,
            "observed_at_ms": 1800.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    assert!(!has_event(&entries, "decodeCandidateStateTransition"));
}

#[test]
fn decoder_local_reset_failed_projects_runtime_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_observation_label": "videoDecoderLocalResetFailed",
        "latest_observation_summary": "reason=stall err=backend unavailable"
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "videoDecoderLocalResetFailed");
    assert_eq!(payload["summary"], "reason=stall err=backend unavailable");
}

#[test]
fn feedback_target_availability_changed_projects_runtime_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_observation_label": "rtcReadIngressObserved",
        "latest_observation_summary": "phase1 rtc read ingress rtp=0 rtcp=0 dc=0 lastDc=none",
        "latest_feedback_target_availability_target": "videoRtcpFeedback",
        "latest_feedback_target_availability_state": "unbound",
        "latest_feedback_target_availability_reason": "videoRtcpFeedbackTargetPending",
        "latest_feedback_target_availability_observed_at_ms": 12.5
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "feedbackTargetAvailabilityChanged");
    assert_eq!(payload["target"], "videoRtcpFeedback");
    assert_eq!(payload["state"], "unbound");
    assert_eq!(payload["reason"], "videoRtcpFeedbackTargetPending");
    assert_eq!(payload["observedAtMs"], 12.5);
    assert_eq!(
        payload["summary"],
        "target=videoRtcpFeedback state=unbound reason=videoRtcpFeedbackTargetPending"
    );
}

#[test]
fn feedback_transport_not_ready_projects_runtime_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_observation_label": "rtcReadIngressObserved",
        "latest_observation_summary": "phase1 rtc read ingress rtp=0 rtcp=0 dc=0 lastDc=none",
        "latest_feedback_target_availability_target": "videoRtcpFeedback",
        "latest_feedback_target_availability_state": "unbound",
        "latest_feedback_target_availability_reason": "videoRtcpFeedbackTransportNotReady",
        "latest_feedback_target_availability_observed_at_ms": 12.5
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "feedbackTargetAvailabilityChanged");
    assert_eq!(payload["target"], "videoRtcpFeedback");
    assert_eq!(payload["state"], "unbound");
    assert_eq!(payload["reason"], "videoRtcpFeedbackTransportNotReady");
    assert_eq!(payload["observedAtMs"], 12.5);
    assert_eq!(
        payload["summary"],
        "target=videoRtcpFeedback state=unbound reason=videoRtcpFeedbackTransportNotReady"
    );
}

#[test]
fn first_frame_latency_continuation_seen_projects_runtime_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_first_frame_latency_observation": {
            "observation_id": 7,
            "episode_id": 906,
            "recovery_epoch": 12,
            "control_ready_to_pli_sent_ms": 20.0,
            "pli_sent_to_first_idr_packet_ms": null,
            "first_idr_packet_to_first_decode_ms": null,
            "first_decode_to_clean_anchor_committed_ms": null,
            "clean_anchor_committed_to_display_stable_ms": null,
            "terminal_phase": "ContinuationSeen",
            "incomplete_reason": "continuationOnlyAwaitingIdr",
            "observed_at_ms": 155.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "firstFrameLatencyObserved");
    assert_eq!(payload["terminalPhase"], "ContinuationSeen");
    assert_eq!(payload["incompleteReason"], "continuationOnlyAwaitingIdr");
    assert_eq!(payload["controlReadyToPliSentMs"], 20.0);
}

#[test]
fn feedback_target_availability_same_semantics_do_not_repeat_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats_first = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_feedback_target_availability_target": "videoRtcpFeedback",
        "latest_feedback_target_availability_state": "ready",
        "latest_feedback_target_availability_reason": "twccSent",
        "latest_feedback_target_availability_observed_at_ms": 10.0
    }));
    let stats_second = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_feedback_target_availability_target": "videoRtcpFeedback",
        "latest_feedback_target_availability_state": "ready",
        "latest_feedback_target_availability_reason": "twccSent",
        "latest_feedback_target_availability_observed_at_ms": 25.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_first);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_second);

    let entries = read_trace_lines(recorder.as_ref());
    let payloads = event_payloads(&entries, "feedbackTargetAvailabilityChanged");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["observedAtMs"], 10.0);
}

#[test]
fn feedback_target_availability_semantic_transition_projects_each_change_once() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats_ready = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_feedback_target_availability_target": "videoRtcpFeedback",
        "latest_feedback_target_availability_state": "ready",
        "latest_feedback_target_availability_reason": "feedbackTargetBound",
        "latest_feedback_target_availability_observed_at_ms": 10.0
    }));
    let stats_unbound = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_feedback_target_availability_target": "videoRtcpFeedback",
        "latest_feedback_target_availability_state": "unbound",
        "latest_feedback_target_availability_reason": "feedbackTargetUnbound",
        "latest_feedback_target_availability_observed_at_ms": 15.0
    }));
    let stats_ready_again = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_feedback_target_availability_target": "videoRtcpFeedback",
        "latest_feedback_target_availability_state": "ready",
        "latest_feedback_target_availability_reason": "feedbackTargetBound",
        "latest_feedback_target_availability_observed_at_ms": 21.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_ready);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_unbound);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats_ready_again);

    let entries = read_trace_lines(recorder.as_ref());
    let payloads = event_payloads(&entries, "feedbackTargetAvailabilityChanged");
    assert_eq!(payloads.len(), 3);
    assert_eq!(payloads[0]["state"], "ready");
    assert_eq!(payloads[0]["reason"], "feedbackTargetBound");
    assert_eq!(payloads[1]["state"], "unbound");
    assert_eq!(payloads[1]["reason"], "feedbackTargetUnbound");
    assert_eq!(payloads[2]["state"], "ready");
    assert_eq!(payloads[2]["reason"], "feedbackTargetBound");
}

#[test]
fn twcc_receiver_mapping_missing_projects_runtime_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_observation_label": "twccReceiverMappingMissing",
        "latest_observation_summary": "mediaSsrc=Some(17493) pendingFeedbackPackets=1 droppedPendingFeedbackTotal=0"
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "twccReceiverMappingMissing");
    assert_eq!(
        payload["summary"],
        "mediaSsrc=Some(17493) pendingFeedbackPackets=1 droppedPendingFeedbackTotal=0"
    );
}

#[test]
fn decoder_bootstrap_gate_rejected_projects_runtime_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_video_decoder_bootstrap_gate_observation": {
            "observation_id": 7,
            "recovery_state": "waiting-keyframe",
            "frame_rtp_timestamp": 2359556541u32,
            "is_idr": false,
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "bootstrap_ready": false,
            "bootstrap_reject_reason": "NonIdrVcl",
            "observed_at_ms": 1400.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "decoderBootstrapGateRejected");
    assert_eq!(payload["observationId"], 7);
    assert_eq!(payload["recoveryState"], "waiting-keyframe");
    assert_eq!(payload["bootstrapRejectReason"], "NonIdrVcl");
    assert_eq!(payload["committedSpsPresent"], true);
    assert_eq!(payload["bootstrapReady"], false);
}

#[test]
fn decode_output_path_observation_projects_runtime_trace_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_decode_output_path_observation": {
            "observation_id": 11,
            "verdict": "backend-no-output",
            "detail": "backendNoOutput",
            "frame_rtp_timestamp": 2359556541u32,
            "is_keyframe": true,
            "status": null,
            "send_packet_status": -11,
            "receive_frame_status": -35,
            "backend_no_output_streak": 4,
            "input_frames_since_last_decoded": 9,
            "bootstrap_reject_reason": null,
            "observed_at_ms": 1416.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "decodeOutputPathObserved");
    assert_eq!(payload["observationId"], 11);
    assert_eq!(payload["verdict"], "backend-no-output");
    assert_eq!(payload["detail"], "backendNoOutput");
    assert_eq!(payload["isKeyframe"], true);
    assert_eq!(payload["sendPacketStatus"], -11);
    assert_eq!(payload["receiveFrameStatus"], -35);
    assert_eq!(payload["backendNoOutputStreak"], 4);
    assert_eq!(payload["inputFramesSinceLastDecoded"], 9);
}

#[test]
fn render_mailbox_state_projects_transition_event() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "latest_render_mailbox_decision": {
            "decision_id": 702,
            "state": "latest-overwrite",
            "action": "replace",
            "detail": "mailboxOverwrite",
            "frame_seq": 77,
            "observed_at_ms": 1801.0
        }
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "renderMailboxStateTransition");
    assert_eq!(payload["decisionId"], 702);
    assert_eq!(payload["state"], "latest-overwrite");
    assert_eq!(payload["detail"], "mailboxOverwrite");
    assert_eq!(payload["frameSeq"], 77);
}

#[test]
fn host_mailbox_state_projects_no_pending_supply_signals() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "present_fps": 59.8,
        "host_mailbox_enqueue_count_total": 301,
        "host_mailbox_drop_count_total": 11,
        "host_mailbox_overwrite_count_total": 9,
        "host_no_pending_take_count_total": 1200,
        "host_no_pending_streak": 66,
        "host_no_pending_max_streak": 132,
        "host_no_pending_pressure_level": "high",
        "present_age_ms": 18.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "hostMailboxState");
    assert_eq!(payload["noPendingTakeCountTotal"], 1200);
    assert_eq!(payload["noPendingStreak"], 66);
    assert_eq!(payload["noPendingMaxStreak"], 132);
    assert_eq!(payload["noPendingPressureLevel"], "high");
    assert_eq!(payload["hostMailboxEnqueueCountTotal"], 301);
    assert_eq!(payload.get("presentEnqueueCountTotal"), None);
}

#[test]
fn host_mailbox_state_projects_cadence_epoch_signals() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "present_fps": 59.8,
        "host_mailbox_enqueue_count_total": 301,
        "host_display_tick_epoch": 4096,
        "host_frame_present_epoch": 3901,
        "host_cadence_phase": "steady",
        "last_displayed_frame_seq": 77,
        "last_displayed_frame_rtp_timestamp": 22334455u32,
        "last_displayed_at_ms": 1440.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "hostMailboxState");
    assert_eq!(payload["hostDisplayTickEpoch"], 4096);
    assert_eq!(payload["hostFramePresentEpoch"], 3901);
    assert_eq!(payload["cadencePhase"], "steady");
    assert_eq!(payload["displayedAgeMs"], serde_json::Value::Null);
    assert_eq!(payload["displayedFrameStale"], false);
    assert_eq!(payload["retainedOldFrameRisk"], false);
    assert_eq!(payload["lastDisplayedFrameSeq"], 77);
    assert_eq!(payload["lastDisplayedFrameRtpTimestamp"], 22334455u32);
    assert_eq!(payload["lastDisplayedAtMs"], 1440.0);
}

#[test]
fn host_mailbox_state_projects_retained_old_frame_risk() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "present_fps": 1.0,
        "host_mailbox_enqueue_count_total": 12,
        "host_mailbox_drop_count_total": 0,
        "host_mailbox_overwrite_count_total": 0,
        "host_no_pending_take_count_total": 9,
        "host_no_pending_streak": 4,
        "host_no_pending_max_streak": 4,
        "host_no_pending_pressure_level": "critical",
        "host_display_tick_epoch": 512,
        "host_frame_present_epoch": 33,
        "host_cadence_phase": "starved",
        "present_age_ms": 486.0,
        "last_displayed_frame_seq": 91,
        "last_displayed_frame_rtp_timestamp": 9988u32,
        "last_displayed_at_ms": 1514.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "hostMailboxState");
    assert_eq!(payload["displayedAgeMs"], 486.0);
    assert_eq!(payload["displayedFrameStale"], true);
    assert_eq!(payload["retainedOldFrameRisk"], true);
    assert_eq!(payload["lastDisplayedFrameSeq"], 91);
}

#[test]
fn host_mailbox_state_records_present_age_transition_from_none_to_fresh() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let first = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "present_fps": 0.0,
        "host_mailbox_enqueue_count_total": 0,
        "host_mailbox_drop_count_total": 0,
        "host_mailbox_overwrite_count_total": 0,
        "host_no_pending_take_count_total": 0,
        "host_no_pending_streak": 0,
        "host_no_pending_max_streak": 0,
        "host_no_pending_pressure_level": "normal",
        "present_age_ms": null
    }));
    let second = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "present_fps": 59.0,
        "host_mailbox_enqueue_count_total": 1,
        "host_mailbox_drop_count_total": 0,
        "host_mailbox_overwrite_count_total": 0,
        "host_no_pending_take_count_total": 1,
        "host_no_pending_streak": 0,
        "host_no_pending_max_streak": 1,
        "host_no_pending_pressure_level": "normal",
        "present_age_ms": 12.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &first);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &second);

    let entries = read_trace_lines(recorder.as_ref());
    let host_rows: Vec<_> = entries
        .iter()
        .filter(|entry| entry["event"] == "hostMailboxState")
        .collect();
    assert_eq!(host_rows.len(), 2);
    assert!(host_rows[0]["payload"]["presentAgeMs"].is_null());
    assert_eq!(host_rows[1]["payload"]["presentAgeMs"], 12.0);
}

#[test]
fn host_frame_present_resumed_emits_transition_event_when_present_epoch_recovers() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let first = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "host_display_tick_epoch": 12,
        "host_frame_present_epoch": 0,
        "host_cadence_phase": "waiting",
        "last_displayed_frame_seq": null,
        "last_displayed_frame_rtp_timestamp": null,
        "last_displayed_at_ms": null
    }));
    let second = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "host_display_tick_epoch": 15,
        "host_frame_present_epoch": 2,
        "host_cadence_phase": "steady",
        "last_displayed_frame_seq": 88,
        "last_displayed_frame_rtp_timestamp": 998877u32,
        "last_displayed_at_ms": 1810.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &first);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &second);

    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "hostFramePresentResumed");
    assert_eq!(payload["previousHostDisplayTickEpoch"], 12);
    assert_eq!(payload["previousHostFramePresentEpoch"], 0);
    assert_eq!(payload["hostDisplayTickEpoch"], 15);
    assert_eq!(payload["hostFramePresentEpoch"], 2);
    assert_eq!(payload["lastDisplayedFrameSeq"], 88);
    assert_eq!(payload["lastDisplayedFrameRtpTimestamp"], 998877u32);
    assert_eq!(payload["lastDisplayedAtMs"], 1810.0);
}

#[test]
fn direct_gaming_state_projects_display_supply_health_and_issue_chain() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "session_phase": "steady",
        "transport_strategy_profile": "cloud",
        "recovery_strategy_profile": "cloud",
        "recovery_diagnosis": "adapterIdleTimeout",
        "direct_gaming_bitrate_band": "steady",
        "runtime_summary": "cloud/steady/steady/displaySupplyStarved",
        "primary_issue_chain": "display:supplyStarved",
        "latest_decision_summary": "owner:supplyStarved:supplyStarved",
        "recovery_owner_state": "supplyStarved",
        "recovery_owner_reason": "supplyStarved",
        "video_owner_source": "supply",
        "video_owner_observed_at_ms": 3200.0,
        "video_health": "displaySupplyStarved",
        "chain_health": "healthy",
        "presentation_health": "displaySupplyStarved",
        "stall_kind": "pipelineStall",
        "host_no_pending_take_count_total": 8062,
        "host_no_pending_streak": 1291,
        "host_no_pending_max_streak": 2866,
        "host_no_pending_pressure_level": "critical",
        "present_age_ms": 1624.0
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let payload = find_event_payload(&entries, "directGamingState");

    assert_eq!(payload["videoHealth"], "displaySupplyStarved");
    assert_eq!(payload["lifecycle"], "steady");
    assert_eq!(payload["streamLifecyclePhase"], "steady");
    assert_eq!(payload["recoveryDiagnosis"], "adapterIdleTimeout");
    assert_eq!(payload["videoOwnerState"], "supplyStarved");
    assert_eq!(payload["videoOwnerReason"], "supplyStarved");
    assert_eq!(payload["videoOwnerSource"], "supply");
    assert_eq!(payload["videoOwnerObservedAtMs"], 3200.0);
    assert_eq!(payload["primaryIssueChain"], "display:supplyStarved");
    assert_eq!(payload["chainHealth"], "healthy");
    assert_eq!(payload["presentationHealth"], "displaySupplyStarved");
    let host_payload = find_event_payload(&entries, "hostMailboxState");
    assert_eq!(host_payload["noPendingPressureLevel"], "critical");
    assert_eq!(host_payload["presentAgeMs"], 1624.0);
}

#[test]
fn observability_snapshot_projects_unified_lifecycle_in_recovery_node() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let stats = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "session_phase": "steady",
        "stream_lifecycle_phase": "recovering",
        "recovery_strategy_profile": "cloud",
        "recovery_diagnosis": "transportAwaitRecoveryAnchor"
    }));

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    let direct_payload = find_event_payload(&entries, "directGamingState");
    assert_eq!(direct_payload["lifecycle"], "recovering");
    assert_eq!(direct_payload["streamLifecyclePhase"], "recovering");

    let snapshot_payload = build_observability_snapshot(&stats);
    assert_eq!(snapshot_payload["recovery"]["lifecycle"], "recovering");
    assert_eq!(
        snapshot_payload["recovery"]["streamLifecyclePhase"],
        "recovering"
    );
    assert_eq!(
        snapshot_payload["recovery"]["diagnosis"],
        "transportAwaitRecoveryAnchor"
    );
}

#[test]
fn direct_gaming_state_emits_transition_when_only_diagnosis_changes() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();

    let baseline = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "session_phase": "recovering",
        "stream_lifecycle_phase": "recovering",
        "recovery_strategy_profile": "cloud",
        "recovery_diagnosis": "transportAwaitRecoveryAnchor",
        "recovery_owner_state": "rebuilding-supply",
        "recovery_owner_reason": "transportAwaitRecoveryAnchor"
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &baseline);

    let changed_diagnosis = test_stats(json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "session_phase": "recovering",
        "stream_lifecycle_phase": "recovering",
        "recovery_strategy_profile": "cloud",
        "recovery_diagnosis": "decoderBackendFailure",
        "recovery_owner_state": "rebuilding-supply",
        "recovery_owner_reason": "transportAwaitRecoveryAnchor"
    }));
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &changed_diagnosis);

    let entries = read_trace_lines(recorder.as_ref());
    let rows = event_payloads(&entries, "directGamingState");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["recoveryDiagnosis"], "transportAwaitRecoveryAnchor");
    assert_eq!(rows[1]["recoveryDiagnosis"], "decoderBackendFailure");
    assert_eq!(rows[1]["videoOwnerReason"], "transportAwaitRecoveryAnchor");
}

#[test]
fn direct_gaming_state_owner_contract_unchanged_does_not_repeat_transition() {
    let recorder = std::sync::Arc::new(
        RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder"),
    );
    let mut state = RuntimeTraceObservationState::default();
    let baseline = json!({
        "resolution": "",
        "rtt": "",
        "fps": 0.0,
        "pl": "0.00%",
        "fl": "",
        "jit": "",
        "br": "",
        "decode": "",
        "session_phase": "steady",
        "transport_strategy_profile": "cloud",
        "recovery_strategy_profile": "cloud",
        "recovery_owner_state": "rebuildingSupply",
        "recovery_owner_reason": "timelineReferenceBroken",
        "video_owner_source": "anchor",
        "video_owner_observed_at_ms": 4096.0,
        "video_health": "recovering",
        "primary_issue_chain": "recovery:timelineReferenceBroken",
        "latest_decision_summary": "owner:rebuildingSupply:timelineReferenceBroken",
        "stall_kind": "waitingKeyframe"
    });
    let stats = test_stats(baseline.clone());

    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);
    let entries = read_trace_lines(recorder.as_ref());
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["event"] == "directGamingState")
            .count(),
        1
    );

    let timeline_changed = test_stats({
        let mut value = baseline;
        value["latest_video_timeline_observation"] = json!({
            "observation_id": 901,
            "source_event": "nack-observation",
            "gap": null,
            "frame": null,
            "chain": {
                "state": "recovering",
                "reason": "timelineReferenceBroken",
                "observed_at_ms": 5000.0
            },
            "observed_at_ms": 5000.0
        });
        value
    });
    record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &timeline_changed);

    let entries = read_trace_lines(recorder.as_ref());
    let latest_direct = entries
        .iter()
        .filter(|entry| entry["event"] == "directGamingState")
        .last()
        .expect("directGamingState entry");
    assert_eq!(
        latest_direct["payload"]["videoOwnerState"],
        "rebuildingSupply"
    );
    assert!(entries
        .iter()
        .any(|entry| entry["event"] == "videoTimelineObserved"));
}

#[test]
fn h264_inspection_snapshot_unlinks_when_frame_rtp_mismatches_keyframe_episode() {
    let stats = test_stats(json!({
        "resolution": "1920x1080",
        "rtt": "0",
        "fps": 60.0,
        "pl": "0",
        "fl": "",
        "jit": "0",
        "br": "0",
        "decode": "",
        "latest_keyframe_request_episode": {
            "episode_id": 1,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "missed",
            "requested_at_ms": 100.0,
            "sent_at_ms": 110.0,
            "deadline_at_ms": 200.0,
            "first_keyframe_packet_at_ms": null,
            "first_keyframe_decoded_at_ms": null,
            "response_rtp_timestamp": 111,
            "response_frame_seq": null,
            "response_verdict": "missed"
        },
        "recent_keyframe_request_episodes": [],
        "latest_h264_inspection_observation": {
            "observation_id": 5,
            "frame_rtp_timestamp": 999999,
            "nal_types": [],
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": true,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "bootstrap_ready": true,
            "admission_accepted": true,
            "observed_at_ms": 500.0,
            "bound_as_recovery_response": false
        }
    }));

    let snapshot = build_observability_snapshot(&stats);
    let h264 = &snapshot["latest"]["h264Inspection"];
    assert_eq!(h264["linkedEpisodeId"], json!(null));
    assert_eq!(h264["linkedEpisodeStatus"], json!(null));
    assert_eq!(h264["isRecoveryKeyframeResponseContext"], false);
}

#[test]
fn h264_inspection_time_window_skips_retired_keyframe_episode() {
    let stats = test_stats(json!({
        "resolution": "1920x1080",
        "rtt": "0",
        "fps": 60.0,
        "pl": "0",
        "fl": "",
        "jit": "0",
        "br": "0",
        "decode": "",
        "latest_keyframe_request_episode": null,
        "recent_keyframe_request_episodes": [{
            "episode_id": 9,
            "request_reason": "transportAwaitRecoveryAnchor",
            "request_kind": "pli",
            "status": "packet-seen",
            "requested_at_ms": 100.0,
            "sent_at_ms": 110.0,
            "deadline_at_ms": 500.0,
            "first_keyframe_packet_at_ms": 120.0,
            "first_keyframe_decoded_at_ms": null,
            "response_rtp_timestamp": 555,
            "response_frame_seq": null,
            "response_verdict": "unknown",
            "retired_at_ms": 400.0
        }],
        "latest_h264_inspection_observation": {
            "observation_id": 3,
            "frame_rtp_timestamp": 999,
            "nal_types": [],
            "has_inband_sps": false,
            "has_inband_pps": false,
            "committed_sps_present": true,
            "committed_pps_present": true,
            "slice_headers_valid": true,
            "delta_continuation_ready": true,
            "parameter_sets_changed": false,
            "config_changed": false,
            "is_idr": false,
            "bootstrap_ready": true,
            "admission_accepted": true,
            "observed_at_ms": 115.0,
            "bound_as_recovery_response": false
        }
    }));

    let snapshot = build_observability_snapshot(&stats);
    let h264 = &snapshot["latest"]["h264Inspection"];
    assert_eq!(h264["linkedEpisodeId"], json!(null));
    assert_eq!(h264["isRecoveryKeyframeResponseContext"], false);
}
