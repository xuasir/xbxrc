use super::super::{RuntimeMediaCounters, RuntimeObservationWindow};
use super::*;

fn sample(at: u64, received: u64, decoded: u64, displayed: u64) -> RuntimeSupervisionSample {
    RuntimeSupervisionSample {
        now_ms: at,
        sampled_at_ms: Some(at),
        sample_id: Some(at.to_string()),
        observation_fresh: true,
        visible: true,
        connection_state: "connected".into(),
        control_channel_state: "open".into(),
        peer_generation: 1,
        connected_elapsed_ms: 20_000,
        counters: Some(RuntimeMediaCounters {
            received: Some(received),
            decoded: Some(decoded),
            presented: Some(decoded),
            packets_received: Some(received * 10),
            ..Default::default()
        }),
        display_observation: Some(RuntimeDisplayObservation {
            source: "drawablePresented".into(),
            active: true,
            surface_ready: true,
            render_epoch: 1,
            surface_revision: 1,
            sampled_at_ms: at,
            submitted: decoded,
            gpu_completed: decoded,
            displayed,
            last_displayed_at_ms: Some(19_990.0),
            last_frame_id: Some(displayed),
            frame_interval_ms: Some(16.7),
            ..Default::default()
        }),
        long: RuntimeObservationWindow {
            received_coverage_ms: 15_000,
            decode_coverage_ms: 15_000,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn decoded_supply_with_stalled_drawable_issues_one_scoped_action() {
    let mut state = DisplayRecoveryState::default();
    let first = state
        .observe(&sample(20_000, 600, 600, 600), true, false)
        .unwrap();
    assert!(!first.eligible);
    let action = state
        .observe(&sample(21_000, 660, 660, 600), true, false)
        .unwrap();
    assert!(action.eligible);
    assert_eq!(action.action.as_ref().unwrap().render_epoch, 1);
    assert_eq!(
        action.action.as_ref().unwrap().reason,
        "drawablePresentationStalled"
    );
    let next = state
        .observe(&sample(22_000, 720, 720, 600), true, false)
        .unwrap();
    assert!(next.active);
    assert!(next.action.is_none());
    let timed_out = state
        .observe(&sample(23_000, 780, 780, 600), true, false)
        .unwrap();
    assert_eq!(timed_out.effect.as_deref(), Some("degraded"));
    assert!(state
        .observe(&sample(24_000, 840, 840, 600), true, false)
        .unwrap()
        .action
        .is_none());
}

#[test]
fn decode_network_background_and_scope_changes_keep_display_action_idle() {
    for scenario in ["decode", "network", "background", "scope", "stale", "proxy"] {
        let mut state = DisplayRecoveryState::default();
        state.observe(&sample(20_000, 600, 600, 600), true, false);
        let mut next = sample(21_000, 660, 660, 600);
        match scenario {
            "decode" => next.counters.as_mut().unwrap().decoded = Some(600),
            "network" => next.counters.as_mut().unwrap().received = Some(600),
            "background" => next.display_observation.as_mut().unwrap().active = false,
            "scope" => next.display_observation.as_mut().unwrap().render_epoch = 2,
            "stale" => next.display_observation.as_mut().unwrap().sampled_at_ms = 20_000,
            "proxy" => {
                let display = next.display_observation.as_mut().unwrap();
                display.source = "gpuProxy".into();
                display.displayed = 660;
                display.last_displayed_at_ms = Some(20_990.0);
            }
            _ => unreachable!(),
        }
        let decision = state.observe(&next, true, false).unwrap();
        assert!(!decision.eligible, "{scenario}");
        assert!(decision.action.is_none(), "{scenario}");
    }
}

#[test]
fn reset_acknowledgement_requires_fresh_frames_and_preserves_episode_budget() {
    let mut state = DisplayRecoveryState::default();
    state.observe(&sample(20_000, 600, 600, 600), true, false);
    let action = state
        .observe(&sample(21_000, 660, 660, 600), true, false)
        .unwrap()
        .action
        .unwrap();
    let mut last = None;
    for at in (22_000..=25_000).step_by(1_000) {
        let count = (at - 21_000) * 60 / 1_000;
        let mut next = sample(at, 660 + count, 660 + count, count);
        let display = next.display_observation.as_mut().unwrap();
        display.render_epoch = 2;
        display.last_frame_id = Some(1_000 + count);
        display.last_displayed_at_ms = Some((at - 10) as f64);
        display.first_displayed_at_ms = Some(21_020.0);
        next.display_recovery_completion = Some(RuntimeDisplayRecoveryCompletion {
            id: action.id,
            result: "acknowledged".into(),
            completed_at_ms: 21_010,
            render_epoch: 2,
        });
        last = state.observe(&next, true, false);
    }
    assert_eq!(last.unwrap().effect.as_deref(), Some("improved"));
    let mut stalled = sample(26_000, 960, 960, 240);
    let display = stalled.display_observation.as_mut().unwrap();
    display.render_epoch = 2;
    display.last_displayed_at_ms = Some(24_990.0);
    let decision = state.observe(&stalled, true, false).unwrap();
    assert!(decision.exhausted);
    assert!(!decision.eligible);
}

#[test]
fn legacy_supervision_json_and_counter_resets_remain_compatible() {
    let legacy: RuntimeSupervisionSample =
        serde_json::from_str(r#"{"nowMs":1000,"visible":true}"#).unwrap();
    assert!(legacy.display_observation.is_none());
    let mut state = DisplayRecoveryState::default();
    assert!(state.observe(&legacy, true, false).is_none());
    state.observe(&sample(20_000, 600, 600, 600), true, false);
    assert!(
        !state
            .observe(&sample(21_000, 1, 1, 1), true, false)
            .unwrap()
            .eligible
    );
}
