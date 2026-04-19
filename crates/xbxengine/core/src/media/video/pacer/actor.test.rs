use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    drive_ready_frames_with_submit, flush_pending_render_output_with_submit,
    format_render_backpressure_summary, next_wait_duration, render_frame_is_stale,
    render_frame_priority, resolve_cadence_sleep_guard_override_ms,
    resolve_host_release_wait_duration, should_replace_render_queue_head, HostCadencePhaseHint,
    HostPacingContext, PendingRenderSubmitResult, PendingRenderSubmitResultWithFrame,
};
use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::media::video::pacer::actor::PacerActorHandle;
use crate::media::video::render::pacer::{
    FramePacingPolicy, HostPacingPressure, QueueHistoryConfig, QueueHistoryController,
};
use crate::media::video::render::renderer::XbxRenderFrame;
use crate::media::video::render::{actor::RendererActorHandle, renderer::XbxRenderState};
use crate::media::video::types::{DecodedFrame, FrameRecoveryDisposition};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::XbxEngineRenderPixelData;

fn make_decoded_frame(frame_seq: u64) -> DecodedFrame {
    DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: frame_seq as u32,
        is_keyframe: frame_seq == 1,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq,
            rendered_at_ms: frame_seq as f64,
            rtp_timestamp: Some(frame_seq as u32),
            is_keyframe: frame_seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([frame_seq as u8; 16]),
            },
        },
    }
}

fn make_keyframe_decoded_frame(frame_seq: u64) -> DecodedFrame {
    let mut frame = make_decoded_frame(frame_seq);
    frame.budget = crate::media::video::ingress::budget::FrameBudgetContext::for_transport(
        crate::media::video::types::FrameValue::new(true, false, 64 * 1024),
        false,
        Some(30.0),
        Some(1_000.0),
        Some(1_050.0),
        false,
        crate::media::video::ingress::budget::FrameBudgetWindowSource::Transport,
    );
    frame.is_keyframe = true;
    frame.surface.is_keyframe = true;
    frame
}

#[test]
fn pending_render_output_keeps_frame_on_backpressure_until_retry_succeeds() {
    let mut render_queue = VecDeque::from([make_decoded_frame(1)]);
    let mut submit_calls = 0usize;

    let first = flush_pending_render_output_with_submit(&mut render_queue, |frame| {
        submit_calls += 1;
        assert_eq!(frame.surface.frame_seq, 1);
        if submit_calls == 1 {
            PendingRenderSubmitResultWithFrame::BackpressureWithFrame(frame)
        } else {
            PendingRenderSubmitResultWithFrame::Submitted(frame)
        }
    });

    assert_eq!(submit_calls, 1);
    assert!(matches!(first, PendingRenderSubmitResult::Backpressure(_)));
    assert_eq!(render_queue.len(), 1);
    assert_eq!(
        render_queue.front().map(|frame| frame.surface.frame_seq),
        Some(1)
    );

    let second = flush_pending_render_output_with_submit(&mut render_queue, |frame| {
        submit_calls += 1;
        assert_eq!(frame.surface.frame_seq, 1);
        PendingRenderSubmitResultWithFrame::Submitted(frame)
    });

    assert_eq!(submit_calls, 2);
    assert!(matches!(second, PendingRenderSubmitResult::Submitted(_)));
    assert!(render_queue.is_empty());
}

#[test]
fn pending_render_output_reports_disconnect_without_silently_requeueing() {
    let mut render_queue = VecDeque::from([make_decoded_frame(7)]);

    let result = flush_pending_render_output_with_submit(&mut render_queue, |frame| {
        assert_eq!(frame.surface.frame_seq, 7);
        PendingRenderSubmitResultWithFrame::Disconnected(frame)
    });

    assert!(matches!(result, PendingRenderSubmitResult::Disconnected(_)));
    assert!(render_queue.is_empty());
}

#[test]
fn render_backpressure_summary_includes_host_epochs_and_queue_depths() {
    let host_context = HostPacingContext {
        host_refresh_interval_ms: 17,
        release_interval_ms: 17,
        host_frame_age_budget_ms: Some(75.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 11,
        present_epoch: 7,
        cadence_phase: HostCadencePhaseHint::Starved,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };

    let summary = format_render_backpressure_summary(3, 1, &host_context);
    assert!(summary.contains("pacingQueueDepth=3"));
    assert!(summary.contains("pendingRenderQueueDepth=1"));
    assert!(summary.contains("hostTickEpoch=11"));
    assert!(summary.contains("presentEpoch=7"));
    assert!(summary.contains("cadencePhase=starved"));
    assert!(summary.contains("hostFrameAgeBudgetMs=75.0"));
}

#[test]
fn render_queue_keeps_existing_higher_priority_frame() {
    let existing = make_keyframe_decoded_frame(101);
    let incoming = make_decoded_frame(102);
    assert!(!should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now()
    ));
    assert!(render_frame_priority(&existing) > render_frame_priority(&incoming));
}

#[test]
fn render_queue_replaces_existing_stale_frame_even_if_priority_is_lower() {
    let mut existing = make_keyframe_decoded_frame(201);
    existing.pts = Instant::now() - Duration::from_millis(20);
    let incoming = make_decoded_frame(202);
    assert!(render_frame_is_stale(&existing, Instant::now()));
    assert!(should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now()
    ));
}

#[test]
fn next_wait_duration_respects_host_release_gate() {
    let policy = FramePacingPolicy::new(16);
    let mut frame = make_decoded_frame(3);
    frame.pts = Instant::now();
    let wait = next_wait_duration(Some(&frame), &policy, false, Some(Duration::from_millis(7)));
    assert_eq!(wait, Duration::from_millis(7));
}

#[test]
fn host_release_gate_disables_itself_when_host_present_is_stale() {
    let context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 0,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Unknown,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let wait = resolve_host_release_wait_duration(&context, 1_100.0, None);
    assert!(wait.is_none());
}

#[test]
fn host_release_gate_waits_until_next_host_tick_window() {
    let context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 0,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Unknown,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let wait = resolve_host_release_wait_duration(&context, 1_006.0, None)
        .expect("host gate should request a wait");
    assert!(wait > Duration::from_millis(7));
    assert!(wait < Duration::from_millis(10));
}

#[test]
fn host_release_gate_prefers_new_display_tick_epoch_over_time_window() {
    let context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 9,
        present_epoch: 4,
        cadence_phase: HostCadencePhaseHint::Steady,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(8));
    assert!(wait.is_none());
}

#[test]
fn host_release_gate_blocks_reusing_same_display_tick_epoch_until_fallback_window() {
    let context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 9,
        present_epoch: 4,
        cadence_phase: HostCadencePhaseHint::Steady,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(9))
        .expect("same tick epoch should still gate release");
    assert!(wait > Duration::from_millis(7));
}

#[test]
fn host_release_gate_blocks_reusing_same_priming_tick_before_first_present() {
    let context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: None,
        display_tick_epoch: 9,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Priming,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(9))
        .expect("priming should block reusing the same host tick before first present");
    assert_eq!(wait, Duration::from_millis(8));
}

#[test]
fn host_release_gate_allows_same_tick_reuse_when_high_refresh_host_lags_present_feedback() {
    let context = HostPacingContext {
        host_refresh_interval_ms: 7,
        release_interval_ms: 33,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 166,
        present_epoch: 5,
        cadence_phase: HostCadencePhaseHint::Steady,
        pressure: HostPacingPressure {
            cadence_phase: HostCadencePhaseHint::Steady,
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: 0,
            present_overwrite_count_total: 0,
            present_submit_count_total: 5,
            present_fps: Some(7.27),
            display_fps: Some(144.0),
        },
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let wait = resolve_host_release_wait_duration(&context, 1_008.0, Some(166));
    assert!(
        wait.is_none(),
        "high-refresh host with lagging present feedback should allow same-tick reuse"
    );
}

#[test]
fn host_release_gate_releases_same_tick_immediately_when_host_is_starved() {
    let context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 9,
        present_epoch: 4,
        cadence_phase: HostCadencePhaseHint::Starved,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(9));
    assert!(wait.is_none());
}

#[test]
fn cadence_sleep_guard_override_shortens_sleep_during_priming() {
    let host_context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: None,
        display_tick_epoch: 1,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Priming,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    assert_eq!(
        resolve_cadence_sleep_guard_override_ms(&host_context),
        Some(8)
    );
}

#[test]
fn cadence_sleep_guard_override_disables_sleep_when_host_is_starved() {
    let host_context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(1_000.0),
        display_tick_epoch: 9,
        present_epoch: 4,
        cadence_phase: HostCadencePhaseHint::Starved,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    assert_eq!(
        resolve_cadence_sleep_guard_override_ms(&host_context),
        Some(0)
    );
}

#[test]
fn drive_ready_frames_holds_due_frame_until_host_release_window_opens() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
        XbxEngineMediaRuntimeStats::default(),
    )));
    let mut pacing_queue = VecDeque::from([make_decoded_frame(11)]);
    let mut render_queue = VecDeque::new();
    let mut queue_history = QueueHistoryController::new(QueueHistoryConfig::default());
    let mut frame_drop_observation_id = 0;
    let mut catch_up_mode = false;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(crate::media::video::decode::video_decode::now_ms_f64()),
        display_tick_epoch: 0,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Unknown,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let pacing_policy = FramePacingPolicy::with_dynamic_budget(
        host_context.release_interval_ms, // 使用release限速间隔
        host_context
            .host_frame_age_budget_ms
            .map(|budget_ms| budget_ms.round() as u64),
        resolve_cadence_sleep_guard_override_ms(&host_context),
        host_context.video_rtt_ms,
        host_context.video_nack_recovery_rtt_ms,
    );
    let mut submit_calls = 0usize;

    let dropped = drive_ready_frames_with_submit(
        &mut pacing_queue,
        &mut render_queue,
        &mut queue_history,
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut catch_up_mode,
        &mut render_backpressure_active,
        |_render_queue| {
            submit_calls += 1;
            PendingRenderSubmitResult::Submitted(make_decoded_frame(1))
        },
    );

    assert!(dropped.is_none());
    assert_eq!(submit_calls, 1);
    assert_eq!(pacing_queue.len(), 1);
    assert!(render_queue.is_empty());
}

#[test]
fn drive_ready_frames_retries_pending_render_output_after_backpressure_clears() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
        XbxEngineMediaRuntimeStats::default(),
    )));
    let mut pacing_queue = VecDeque::from([make_decoded_frame(21)]);
    let mut render_queue = VecDeque::new();
    let mut queue_history = QueueHistoryController::new(QueueHistoryConfig::default());
    let mut frame_drop_observation_id = 0;
    let mut catch_up_mode = false;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        host_refresh_interval_ms: 16,
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        latest_host_present_time_ms: Some(
            crate::media::video::decode::video_decode::now_ms_f64() - 64.0,
        ),
        display_tick_epoch: 0,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Unknown,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let pacing_policy = FramePacingPolicy::with_dynamic_budget(
        host_context.release_interval_ms, // 使用release限速间隔
        host_context
            .host_frame_age_budget_ms
            .map(|budget_ms| budget_ms.round() as u64),
        resolve_cadence_sleep_guard_override_ms(&host_context),
        host_context.video_rtt_ms,
        host_context.video_nack_recovery_rtt_ms,
    );
    let mut submitted_seqs = Vec::new();
    let mut flush_calls = 0usize;

    let first = drive_ready_frames_with_submit(
        &mut pacing_queue,
        &mut render_queue,
        &mut queue_history,
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut catch_up_mode,
        &mut render_backpressure_active,
        |render_queue| {
            flush_calls += 1;
            if render_queue.is_empty() {
                return PendingRenderSubmitResult::Submitted(make_decoded_frame(0));
            }
            let frame = render_queue
                .pop_front()
                .expect("render queue should contain frame during flush");
            render_queue.push_front(frame.clone());
            PendingRenderSubmitResult::Backpressure(frame)
        },
    );

    assert!(first.is_none());
    assert!(pacing_queue.is_empty());
    assert_eq!(render_queue.len(), 1);
    assert!(render_backpressure_active);
    assert_eq!(flush_calls, 2);

    let second = drive_ready_frames_with_submit(
        &mut pacing_queue,
        &mut render_queue,
        &mut queue_history,
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut catch_up_mode,
        &mut render_backpressure_active,
        |render_queue| {
            let frame = render_queue
                .pop_front()
                .expect("render queue should still contain pending frame");
            submitted_seqs.push(frame.surface.frame_seq);
            PendingRenderSubmitResult::Submitted(frame)
        },
    );

    assert!(second.is_none());
    assert!(render_queue.is_empty());
    assert_eq!(submitted_seqs, vec![21]);
    assert!(!render_backpressure_active);
}

#[test]
fn host_cadence_gate_blocks_then_releases_frame_to_renderer() {
    let runtime_stats = Arc::new(std::sync::Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let now_ms = crate::media::video::decode::video_decode::now_ms_f64();
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.host_display_interval_ms = Some(80.0);
        stats.latest_video_host_present_time_ms = Some(now_ms);
    }
    let render_state = Arc::new(std::sync::Mutex::new(XbxRenderState::default()));
    let renderer = Arc::new(RendererActorHandle::new(
        render_state.clone(),
        runtime_stats.clone(),
    ));
    let pacer = PacerActorHandle::new(renderer.clone(), runtime_stats.clone(), 16);

    let mut frame = make_decoded_frame(42);
    frame.pts = Instant::now();
    pacer.submit(frame).expect("submit frame to pacer");

    std::thread::sleep(Duration::from_millis(8));
    let early = render_state
        .lock()
        .expect("render state lock")
        .take_latest_frame();
    assert!(
        early.is_none(),
        "host cadence gate should block early release"
    );

    let deadline = Instant::now() + Duration::from_millis(120);
    let mut released_seq = None;
    while Instant::now() < deadline {
        let frame = render_state
            .lock()
            .expect("render state lock")
            .take_latest_frame();
        if frame.is_some() {
            released_seq = frame.map(|f| f.frame_seq);
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(released_seq, Some(42));
    pacer.stop();
    renderer.stop();
}
