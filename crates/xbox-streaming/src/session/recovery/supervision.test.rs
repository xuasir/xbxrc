use super::*;

fn local_sample(at: u64, decoded: u64, submitted: u64) -> RuntimeSupervisionSample {
    let mut result = sample(at, if decoded > 0 { 60.0 } else { 0.0 });
    result.has_presented_media = true;
    result.frame_interval_ms = Some(1000.0 / 60.0);
    result.inactivity_elapsed_ms = 0;
    result.renderer_kind = Some("webgl2".into());
    result.renderer_generation = Some(1);
    result.renderer_submitted_total = Some(submitted);
    result.renderer_submit_age_ms = Some(at.saturating_sub(20_000) as f64);
    result.local_counters = Some(RuntimeMediaCounters {
        received: Some(at.saturating_sub(20_000) * 60 / 1_000),
        decoded: Some(decoded),
        presented: Some(0),
        packets_received: Some(at.saturating_sub(20_000)),
        packets_lost: None,
    });
    result
}

#[test]
fn local_stall_starts_fresh_evidence_after_a_visibility_boundary() {
    let mut state = RuntimeSupervisionState::default();
    for at in (20_000..=22_000).step_by(1_000) {
        decide_runtime_supervision(&local_sample(at, 0, 0), &mut state);
    }
    for at in [22_200, 23_000, 24_000] {
        let mut input = local_sample(at, 0, 0);
        input.eligible_since_ms = 22_250;
        let decision = decide_runtime_supervision(&input, &mut state);
        if at < 24_000 {
            assert!(decision.local_stall.is_none());
        } else {
            assert_eq!(decision.local_stall.unwrap().coverage_ms, 1_000);
        }
    }
}

#[test]
fn local_stall_recovers_30_and_60_fps_decode_after_two_seconds_of_fresh_supply() {
    for interval in [1000.0 / 30.0, 1000.0 / 60.0] {
        let mut state = RuntimeSupervisionState::default();
        for at in (20_000..=22_000).step_by(1_000) {
            let mut input = local_sample(at, 0, 0);
            input.frame_interval_ms = Some(interval);
            let decision = decide_runtime_supervision(&input, &mut state);
            if at < 22_000 {
                assert!(decision.local_stall.is_none());
            } else {
                let local = decision.local_stall.unwrap();
                assert!(local.eligible);
                assert_eq!(local.domain, "decode");
                assert_eq!(local.action, "keyframeRequest");
                assert_eq!(local.age_ms, 2_000);
                assert!(!decision.decode_recovery_eligible);
                assert!(!decision.media_connection_eligible);
            }
        }
    }
}

#[test]
fn local_stall_separates_renderer_stall_from_sparse_callbacks_and_reset_facts() {
    let mut state = RuntimeSupervisionState::default();
    for at in (20_000..=22_000).step_by(1_000) {
        let decision = decide_runtime_supervision(&local_sample(at, at - 20_000, 0), &mut state);
        if at == 22_000 {
            let local = decision.local_stall.unwrap();
            assert_eq!(local.domain, "presentation");
            assert_eq!(local.action, "playbackRebuild");
        }
    }
    let sparse = local_sample(23_000, 3_000, 60);
    assert!(decide_runtime_supervision(&sparse, &mut state)
        .local_stall
        .is_none());
    let mut reset = local_sample(24_000, 0, 0);
    reset.video_generation = 2;
    assert!(decide_runtime_supervision(&reset, &mut state)
        .local_stall
        .is_none());
    reset.visible = false;
    assert!(decide_runtime_supervision(&reset, &mut state)
        .local_stall
        .is_none());
    reset.visible = true;
    reset.now_ms = 25_000;
    reset.sampled_at_ms = Some(25_000);
    reset.sample_id = Some("after-visible".into());
    assert!(decide_runtime_supervision(&reset, &mut state)
        .local_stall
        .is_none());
}

#[test]
fn local_stall_upgrades_an_ineffective_keyframe_once_and_preserves_same_action_cooling() {
    let mut state = RuntimeSupervisionState::default();
    for at in (20_000..=25_000).step_by(1_000) {
        let mut input = local_sample(at, 0, 0);
        if at >= 23_000 {
            input.previous_action_at_ms = Some(22_000);
            input.previous_action_kind = Some("keyframeRequest".into());
        }
        if at == 25_000 {
            input.previous_action_effect = Some("neutral".into());
        }
        let decision = decide_runtime_supervision(&input, &mut state);
        if at == 24_000 {
            assert!(!decision.local_stall.as_ref().unwrap().eligible);
        }
        if at == 25_000 {
            let local = decision.local_stall.unwrap();
            assert!(local.eligible);
            assert_eq!(local.action, "playbackRebuild");
        }
    }
    let mut input = local_sample(26_000, 0, 0);
    input.previous_action_at_ms = Some(25_000);
    input.previous_action_kind = Some("playbackRebuild".into());
    input.previous_action_effect = Some("neutral".into());
    assert!(
        !decide_runtime_supervision(&input, &mut state)
            .local_stall
            .unwrap()
            .eligible
    );
}

#[test]
fn local_stall_renderer_probe_closes_on_new_renderer_submission_with_the_same_video() {
    let mut input = local_sample(23_000, 60, 60);
    input.renderer_generation = Some(2);
    input.renderer_present_fps = Some(60.0);
    input.renderer_present_coverage_ms = Some(3_000);
    input.renderer_submit_age_ms = Some(20.0);
    input.short.video_coverage_ms = 0;
    let probe = RuntimeRecoveryProbe {
        action: "playbackRebuild".into(),
        started_at_ms: 20_000,
        observe_ms: 3_000,
        required_observe_ms: 3_000,
        sample_id: Some("before-renderer".into()),
        video_generation: Some(1),
        peer_generation: Some(1),
        renderer_generation: Some(1),
        recovery_scope: Some("renderer".into()),
        baseline: RuntimeRecoveryBaseline {
            decode_fps: Some(60.0),
            video_fps: Some(0.0),
        },
    };
    assert_eq!(
        evaluate_runtime_recovery_probe(&probe, &input, true)
            .unwrap()
            .result,
        "improved"
    );
    input.renderer_generation = Some(1);
    assert!(evaluate_runtime_recovery_probe(&probe, &input, true).is_none());
}

fn delay_sample(at: u64, rtt: f64, first: f64, last: f64) -> RuntimeSupervisionSample {
    let mut result = quality_sample(at, 0.0);
    result.rtt_ms = Some(rtt);
    result.short.rtt_coverage_ms = SOFT_WINDOW_MS;
    result.short.rtt_ms_avg = Some(rtt);
    result.short.jitter_buffer_coverage_ms = SOFT_WINDOW_MS;
    result.short.jitter_buffer_ms_first = Some(first);
    result.short.jitter_buffer_ms_last = Some(last);
    result.long.rtt_coverage_ms = STABLE_WINDOW_MS;
    result.long.rtt_ms_avg = Some(rtt);
    result
}

#[test]
fn low_loss_delay_pressure_uses_continuous_rtt_and_buffer_evidence() {
    let mut window = delay_sample(3_000, 180.0, 20.0, 20.0).short;
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(30.0)),
        "warning"
    );
    assert_eq!(
        assess_runtime_network_pressure(&window, Some(30.0)).reason,
        "delayAccumulation"
    );
    window.jitter_buffer_ms_last = Some(160.0);
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(30.0)),
        "congested"
    );
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(180.0)),
        "stable"
    );
    window.rtt_coverage_ms = 1_000;
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(30.0)),
        "stable"
    );
    window.loss_avg = None;
    window.rtt_ms_avg = Some(f64::NAN);
    assert_eq!(
        classify_runtime_network_pressure(&window, Some(30.0)),
        "unknown"
    );
}

#[test]
fn delay_quality_effect_requires_improved_delay_and_preserved_supply() {
    for improved in [true, false] {
        let mut state = RuntimeSupervisionState::default();
        decide_runtime_supervision(&delay_sample(3_000, 30.0, 20.0, 20.0), &mut state);
        let mut action = None;
        for at in (4_000..=7_000).step_by(1_000) {
            action = decide_runtime_supervision(&delay_sample(at, 180.0, 20.0, 160.0), &mut state)
                .quality
                .action
                .or(action);
        }
        let action = action.unwrap();
        assert_eq!(action.reason, "delayAccumulation");
        for at in (8_000..=11_000).step_by(1_000) {
            let mut post = delay_sample(
                at,
                if improved { 80.0 } else { 180.0 },
                if improved { 60.0 } else { 160.0 },
                if improved { 60.0 } else { 160.0 },
            );
            post.quality_level = 1;
            post.quality_completion = Some(RuntimeQualityCompletion {
                id: action.id,
                result: "applied".into(),
                previous_level: 0,
                completed_at_ms: 8_000,
            });
            let decision = decide_runtime_supervision(&post, &mut state);
            assert!(decision.quality.action.is_none());
            if at == 11_000 {
                assert_eq!(
                    decision.quality.effect.as_deref(),
                    Some(if improved { "improved" } else { "neutral" })
                );
                assert_eq!(decision.quality.rtt_baseline_ms, Some(30.0));
            }
        }
        if !improved {
            for at in (12_000..=23_000).step_by(1_000) {
                let mut post = delay_sample(at, 180.0, 160.0, 160.0);
                post.quality_level = 1;
                let decision = decide_runtime_supervision(&post, &mut state);
                if at == 23_000 {
                    assert_eq!(
                        decision.quality.action.unwrap().reason,
                        "ineffectiveRestore"
                    );
                } else {
                    assert!(decision.quality.action.is_none());
                }
            }
        }
    }
}

#[test]
fn delay_warning_preserves_low_baseline_until_path_changes() {
    let mut state = RuntimeSupervisionState::default();
    decide_runtime_supervision(&delay_sample(3_000, 30.0, 20.0, 20.0), &mut state);
    for at in (4_000..=30_000).step_by(1_000) {
        let decision = decide_runtime_supervision(&delay_sample(at, 180.0, 20.0, 20.0), &mut state);
        assert_eq!(decision.quality.rtt_baseline_ms, Some(30.0));
        assert_eq!(decision.quality.state, "warning");
        assert!(decision.quality.action.is_none());
    }
    let mut changed = delay_sample(31_000, 180.0, 20.0, 20.0);
    changed.path_id = Some("changed-path".into());
    let decision = decide_runtime_supervision(&changed, &mut state);
    assert_eq!(decision.quality.rtt_baseline_ms, Some(180.0));
    assert_eq!(decision.quality.state, "stable");
}

#[test]
fn cumulative_windows_include_observed_jitter_buffer_and_clear_on_counter_reset() {
    let mut windows = RuntimeWindowAccumulator::default();
    for second in 0..=3 {
        let counters = RuntimeMediaCounters {
            received: Some(second * 60),
            decoded: Some(second * 60),
            presented: Some(second * 60),
            packets_received: Some(second * 100),
            packets_lost: Some(0),
        };
        windows.observe(
            second * 1000,
            &counters,
            Some(10.0),
            Some(2.0),
            Some(second as f64 * 20.0),
        );
    }
    let window = windows.window(3000, SOFT_WINDOW_MS);
    assert_eq!(window.jitter_buffer_coverage_ms, 3000);
    assert_eq!(window.jitter_buffer_ms_first, Some(20.0));
    assert_eq!(window.jitter_buffer_ms_last, Some(60.0));
    assert_eq!(window.decode_fps_avg, Some(60.0));
    windows.observe(4000, &RuntimeMediaCounters::default(), None, None, None);
    assert_eq!(
        windows
            .window(4000, SOFT_WINDOW_MS)
            .jitter_buffer_coverage_ms,
        2000
    );
    windows.observe(
        5000,
        &RuntimeMediaCounters {
            received: Some(240),
            ..Default::default()
        },
        None,
        None,
        Some(f64::NAN),
    );
    windows.observe(
        6000,
        &RuntimeMediaCounters {
            received: Some(0),
            ..Default::default()
        },
        None,
        None,
        Some(20.0),
    );
    let window = windows.window(6000, SOFT_WINDOW_MS);
    assert_eq!(window.jitter_buffer_coverage_ms, 0);
    assert_eq!(window.jitter_buffer_ms_first, None);
}

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
        "warning"
    );
    window.rtt_coverage_ms = 2_000;
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
        renderer_generation: None,
        recovery_scope: None,
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
