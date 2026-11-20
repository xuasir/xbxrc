use super::*;

#[test]
fn same_facts_replay_pc_decode_and_network_baseline() {
    let baseline: serde_json::Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
        "/../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/shared-supervision-baseline.json"))).unwrap();
    for scenario in baseline["scenarios"].as_array().unwrap() {
        let mut state = RuntimeSupervisionState::default();
        for row in scenario["rows"].as_array().unwrap() {
            let sample: RuntimeSupervisionSample =
                serde_json::from_value(row["sample"].clone()).unwrap();
            let result = decide_runtime_supervision(&sample, &mut state);
            let actual = serde_json::json!({
                "decodeRecoveryEligible": result.decode_recovery_eligible,
                "healthyEpisodeComplete": result.healthy_episode_complete,
                "networkPressure": result.network_pressure,
                "qualityState": result.quality.state,
                "qualityEffect": result.quality.effect,
                "qualityAction": result.quality.action,
            });
            assert_eq!(
                actual, row["expected"],
                "scenario={} at={}",
                scenario["name"], sample.now_ms
            );
        }
    }
}

fn sample(at: u64, decode: f64) -> RuntimeSupervisionSample {
    let short = RuntimeObservationWindow {
        received_coverage_ms: SOFT_WINDOW_MS,
        decode_coverage_ms: SOFT_WINDOW_MS,
        video_coverage_ms: SOFT_WINDOW_MS,
        inbound_fps_avg: Some(60.0),
        decode_fps_avg: Some(decode),
        video_present_fps_avg: Some(decode),
        loss_coverage_ms: SOFT_WINDOW_MS,
        loss_avg: Some(0.0),
        ..Default::default()
    };
    RuntimeSupervisionSample {
        now_ms: at,
        sampled_at_ms: Some(at),
        sample_id: Some(at.to_string()),
        observation_fresh: true,
        visible: true,
        connection_state: "connected".into(),
        control_channel_state: "open".into(),
        connected_elapsed_ms: at,
        inactivity_elapsed_ms: at,
        inbound_video_fps: Some(60.0),
        decode_fps: Some(decode),
        peer_generation: 1,
        video_generation: 1,
        video_present_age_ms: Some(if decode > 0.0 { 20.0 } else { 10_000.0 }),
        long: RuntimeObservationWindow {
            received_coverage_ms: STABLE_WINDOW_MS,
            decode_coverage_ms: STABLE_WINDOW_MS,
            video_coverage_ms: STABLE_WINDOW_MS,
            ..short.clone()
        },
        short,
        ..Default::default()
    }
}

#[test]
fn shared_thresholds_preserve_pc_startup_default_and_frame_interval_limits() {
    assert_eq!(FIRST_FRAME_GUARD_MS, 4_000);
    assert_eq!(runtime_stall_threshold(14_999, Some(16.0)), 8_000);
    assert_eq!(runtime_stall_threshold(15_000, None), 5_000);
    assert_eq!(runtime_stall_threshold(15_000, Some(16.0)), 1_500);
    assert_eq!(runtime_stall_threshold(15_000, Some(600.0)), 7_000);
    assert_eq!(runtime_stall_threshold(15_000, Some(f64::NAN)), 5_000);
}

#[test]
fn continuous_decode_pressure_requires_new_covered_samples_and_action_cooling() {
    let mut state = RuntimeSupervisionState::default();
    for at in 3_000..=6_000 {
        let decision = decide_runtime_supervision(&sample(at, 0.0), &mut state);
        assert_eq!(decision.decode_recovery_eligible, at == 6_000);
    }
    assert!(decide_runtime_supervision(&sample(6_000, 0.0), &mut state).duplicate);
    let mut next = sample(7_000, 0.0);
    next.previous_action_at_ms = Some(6_000);
    assert!(!decide_runtime_supervision(&next, &mut state).decode_recovery_eligible);
}

#[test]
fn gap_visibility_and_old_peer_preserve_evidence_boundaries() {
    let mut state = RuntimeSupervisionState::default();
    decide_runtime_supervision(&sample(3_000, 0.0), &mut state);
    assert!(!decide_runtime_supervision(&sample(6_000, 0.0), &mut state).decode_recovery_eligible);
    let mut old = sample(7_000, 0.0);
    old.peer_generation = 0;
    assert!(!decide_runtime_supervision(&old, &mut state).fresh);
    assert_eq!(state.sampled_at_ms, Some(6_000));
    let mut hidden = sample(7_000, 0.0);
    hidden.visible = false;
    assert!(!decide_runtime_supervision(&hidden, &mut state).fresh);
    assert!(!decide_runtime_supervision(&sample(8_000, 0.0), &mut state).decode_recovery_eligible);
}

#[test]
fn inbound_progress_separates_transport_loss_from_local_decode_stall() {
    let mut state = RuntimeSupervisionState::default();
    let mut first = sample(3_000, 0.0);
    first.inactivity_elapsed_ms = 8_000;
    assert!(!decide_runtime_supervision(&first, &mut state).has_inbound_progress);
    let moving = decide_runtime_supervision(&sample(4_000, 0.0), &mut state);
    assert!(moving.has_inbound_progress);
    assert!(!moving.media_connection_eligible);
    let mut empty = sample(5_000, 0.0);
    empty.inbound_video_fps = Some(0.0);
    assert!(decide_runtime_supervision(&empty, &mut state).media_connection_eligible);
    empty.now_ms = 6_000;
    empty.sampled_at_ms = Some(5_000);
    empty.sample_id = Some("old".into());
    assert!(!decide_runtime_supervision(&empty, &mut state).media_connection_eligible);
}

#[test]
fn network_pressure_uses_loss_and_growth_with_full_coverage() {
    let mut window = sample(3_000, 60.0).short;
    window.loss_avg = Some(0.03);
    window.rtt_coverage_ms = 3_000;
    window.rtt_ms_avg = Some(300.0);
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(300.0)),
        "warning"
    );
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(100.0)),
        "congested"
    );
    window.loss_coverage_ms = 2_000;
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(100.0)),
        "unknown"
    );
}

#[test]
fn recovery_effect_requires_fresh_coverage_and_correct_generations() {
    let mut sample = sample(7_000, 60.0);
    let probe = RuntimeRecoveryProbe {
        action: "keyframeRequest".into(),
        started_at_ms: 3_000,
        observe_ms: 3_000,
        required_observe_ms: 3_000,
        sample_id: Some("3000".into()),
        peer_generation: Some(1),
        video_generation: Some(1),
        baseline: RuntimeRecoveryBaseline {
            decode_fps: Some(0.0),
            video_fps: Some(0.0),
        },
    };
    assert_eq!(
        evaluate_runtime_recovery_probe(&probe, &sample, true)
            .unwrap()
            .result,
        "improved"
    );
    assert!(evaluate_runtime_recovery_probe(&probe, &sample, false).is_none());
    sample.now_ms = 18_000;
    sample.peer_generation = 2;
    assert_eq!(
        evaluate_runtime_recovery_probe(&probe, &sample, true)
            .unwrap()
            .result,
        "unknown"
    );
    let mut delayed = probe;
    delayed.required_observe_ms = 20_000;
    assert_eq!(
        evaluate_runtime_recovery_probe(&delayed, &sample, true)
            .unwrap()
            .reason,
        "pathResponseExceedsDeadline"
    );
}

#[test]
fn native_counters_have_continuous_windows_and_clear_on_regression() {
    let mut state = RuntimeSupervisionState::default();
    for at in (1_000..=7_000).step_by(1_000) {
        let mut native = sample(at, 0.0);
        native.short = Default::default();
        native.long = Default::default();
        native.counters = Some(RuntimeMediaCounters {
            received: Some(at * 60 / 1_000),
            decoded: Some(0),
            presented: Some(0),
            packets_received: Some(at * 3),
            packets_lost: Some(0),
        });
        let decision = decide_runtime_supervision(&native, &mut state);
        assert_eq!(decision.short.received_coverage_ms, (at - 1_000).min(3_000));
        assert_eq!(decision.decode_recovery_eligible, at >= 7_000);
    }
    let mut restarted = sample(8_000, 0.0);
    restarted.counters = Some(RuntimeMediaCounters {
        received: Some(1),
        decoded: Some(0),
        ..Default::default()
    });
    let decision = decide_runtime_supervision(&restarted, &mut state);
    assert_eq!(decision.short.received_coverage_ms, 0);
    assert!(!decision.decode_recovery_eligible);
}

fn quality_sample(at: u64, loss: f64) -> RuntimeSupervisionSample {
    let mut result = sample(at, 60.0);
    result.short.loss_avg = Some(loss);
    result.long.loss_avg = Some(loss);
    result.long.loss_coverage_ms = STABLE_WINDOW_MS;
    result.quality_supported = true;
    result.path_id = Some("same-path".into());
    result
}

#[test]
fn quality_reservation_is_once_and_success_uses_post_action_network_supply() {
    let mut state = RuntimeSupervisionState::default();
    for at in (3_000..6_000).step_by(1_000) {
        assert!(
            decide_runtime_supervision(&quality_sample(at, 0.1), &mut state)
                .quality
                .action
                .is_none()
        );
    }
    let action = decide_runtime_supervision(&quality_sample(6_000, 0.1), &mut state)
        .quality
        .action
        .unwrap();
    assert_eq!(action.level, 1);
    assert_eq!(action.reason, "sustainedCongestion");
    assert!(
        decide_runtime_supervision(&quality_sample(6_000, 0.1), &mut state)
            .quality
            .action
            .is_none()
    );
    assert!(
        decide_runtime_supervision(&quality_sample(7_000, 0.1), &mut state)
            .quality
            .action
            .is_none()
    );
    for at in (8_000..=11_000).step_by(1_000) {
        let mut post = quality_sample(at, 0.01);
        post.quality_level = 1;
        post.quality_completion = Some(RuntimeQualityCompletion {
            id: action.id,
            result: "applied".into(),
            previous_level: 0,
            completed_at_ms: 8_000,
        });
        let result = decide_runtime_supervision(&post, &mut state);
        assert!(result.quality.action.is_none());
        if at == 11_000 {
            assert_eq!(result.quality.effect.as_deref(), Some("improved"));
        }
    }
    let mut next = quality_sample(12_000, 0.01);
    next.quality_level = 1;
    assert_eq!(
        decide_runtime_supervision(&next, &mut state).quality.reason,
        "pathStable"
    );
}

#[test]
fn ineffective_quality_returns_to_previous_level_and_missing_supply_is_unknown() {
    let mut state = RuntimeSupervisionState::default();
    let mut action = None;
    for at in (3_000..=6_000).step_by(1_000) {
        action = decide_runtime_supervision(&quality_sample(at, 0.1), &mut state)
            .quality
            .action
            .or(action);
    }
    let action = action.unwrap();
    for at in (7_000..=22_000).step_by(1_000) {
        let mut post = quality_sample(at, 0.01);
        post.quality_level = 1;
        post.short.inbound_fps_avg = Some(10.0);
        post.quality_completion = Some(RuntimeQualityCompletion {
            id: action.id,
            result: "applied".into(),
            previous_level: 0,
            completed_at_ms: 7_000,
        });
        let result = decide_runtime_supervision(&post, &mut state);
        if at == 10_000 {
            assert_eq!(result.quality.effect.as_deref(), Some("unknown"));
        }
        if at == 22_000 {
            let restore = result.quality.action.unwrap();
            assert_eq!(restore.level, 0);
            assert_eq!(restore.reason, "ineffectiveRestore");
        }
    }
}

#[test]
fn unsupported_quality_retains_level_and_does_not_repeat_requests() {
    let mut state = RuntimeSupervisionState::default();
    let mut action = None;
    for at in (3_000..=6_000).step_by(1_000) {
        action = decide_runtime_supervision(&quality_sample(at, 0.1), &mut state)
            .quality
            .action
            .or(action);
    }
    let action = action.unwrap();
    for at in (7_000..=40_000).step_by(1_000) {
        let mut rejected = quality_sample(at, 0.1);
        rejected.quality_completion = Some(RuntimeQualityCompletion {
            id: action.id,
            result: "unsupported".into(),
            previous_level: 0,
            completed_at_ms: 7_000,
        });
        assert!(decide_runtime_supervision(&rejected, &mut state)
            .quality
            .action
            .is_none());
    }
}

#[test]
fn connection_episode_budget_survives_peer_replacement_and_resets_after_new_frames() {
    let mut state = RuntimeSupervisionState::default();
    let mut request = sample(3_000, 0.0);
    request.request_connection_recovery = true;
    let decision = decide_runtime_supervision(&request, &mut state);
    assert!(decision.connection_recovery_allowed);
    assert!(!decide_runtime_supervision(&request, &mut state).connection_recovery_allowed);
    request.now_ms = 4_000;
    request.connection_recovery_followup = true;
    assert!(decide_runtime_supervision(&request, &mut state).connection_recovery_allowed);
    request.now_ms = 5_000;
    request.peer_generation = 2;
    assert!(decide_runtime_supervision(&request, &mut state).connection_recovery_exhausted);
    for at in (6_000..=11_000).step_by(1_000) {
        let mut healthy = sample(at, 60.0);
        healthy.peer_generation = 2;
        healthy.connection_recovery_action_completed = true;
        decide_runtime_supervision(&healthy, &mut state);
    }
    request.now_ms = 12_000;
    assert!(decide_runtime_supervision(&request, &mut state).connection_recovery_allowed);
}

#[test]
fn connection_arbiter_matches_fact_reason_window_and_observation_barrier() {
    let mut state = RuntimeSupervisionState::default();
    let mut request = sample(3000, 0.0);
    request.request_connection_recovery = true;
    request.connection_state = "failed".into();
    request.connection_recovery_fact_key = Some("transportConnectionState:failed".into());
    assert!(decide_runtime_supervision(&request, &mut state).connection_recovery_allowed);
    request.now_ms = 4000;
    assert!(!decide_runtime_supervision(&request, &mut state).connection_recovery_allowed);
    request.connection_state = "closed".into();
    request.connection_recovery_fact_key = Some("transportConnectionState:closed".into());
    assert!(decide_runtime_supervision(&request, &mut state).connection_recovery_allowed);
    request.connection_recovery_action_completed = true;
    let observing = decide_runtime_supervision(&request, &mut state);
    assert!(!observing.connection_recovery_allowed);
    assert!(!observing.connection_recovery_exhausted);
    request.now_ms = 123000;
    assert!(decide_runtime_supervision(&request, &mut state).connection_recovery_exhausted);
}

#[test]
fn missing_native_decode_counter_keeps_decode_pressure_unknown() {
    let mut state = RuntimeSupervisionState::default();
    for at in (1000..=20000).step_by(1000) {
        let mut native = sample(at, 0.0);
        native.counters = Some(RuntimeMediaCounters {
            received: Some(at * 60 / 1000),
            decoded: None,
            presented: Some(0),
            packets_received: Some(at * 3),
            packets_lost: Some(0),
        });
        let result = decide_runtime_supervision(&native, &mut state);
        assert_eq!(result.short.decode_coverage_ms, 0);
        assert!(!result.decode_recovery_eligible);
    }
}
