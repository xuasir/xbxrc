use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    drive_ready_frames_with_submit, enqueue_render_frame, flush_pending_render_output_with_submit,
    format_render_backpressure_summary, next_wait_duration, render_frame_is_stale,
    resolve_cadence_sleep_guard_override_ms, resolve_host_pacing_context,
    should_replace_render_queue_head, HostCadencePhaseHint, HostPacingContext,
    PendingRenderSubmitResult, PendingRenderSubmitResultWithFrame,
};
use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::media::video::ingress::budget::FrameBudgetWindowSource;
use crate::media::video::pacer::actor::PacerActorHandle;
use crate::media::video::render::pacer::{FramePacingPolicy, HostPacingPressure};
use crate::media::video::render::renderer::XbxRenderFrame;
use crate::media::video::render::{actor::RendererActorHandle, renderer::XbxRenderState};
use crate::media::video::types::{DecodedFrame, FrameRecoveryDisposition};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::XbxEngineRenderPixelData;

fn make_decoded_frame(frame_seq: u64) -> DecodedFrame {
    DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: frame_seq as u32,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: frame_seq == 1,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq,
            rendered_at_ms: frame_seq as f64,
            rtp_timestamp: Some(frame_seq as u32),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: frame_seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([frame_seq as u8; 16]),
            },
        },
    }
}

fn make_recovery_window_decoded_frame(frame_seq: u64) -> DecodedFrame {
    let mut frame = make_decoded_frame(frame_seq);
    frame.budget = crate::media::video::ingress::budget::FrameBudgetContext::for_transport(
        crate::media::video::types::FrameValue::new(frame_seq == 1, false, 1024),
        false,
        Some(30.0),
        Some(1_000.0),
        Some(1_016.0),
        false,
        FrameBudgetWindowSource::Recovery,
    );
    frame
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

fn make_decoded_frame_with_epoch(frame_seq: u64, recovery_epoch_tag: Option<u64>) -> DecodedFrame {
    let mut frame = make_decoded_frame(frame_seq);
    frame.recovery_epoch_tag = recovery_epoch_tag;
    frame.surface.recovery_epoch_tag = recovery_epoch_tag;
    frame
}

#[test]
fn higher_recovery_epoch_replaces_render_queue_head_even_with_lower_seq() {
    let existing = make_decoded_frame_with_epoch(200, Some(3));
    let incoming = make_decoded_frame_with_epoch(150, Some(4));

    assert!(should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now(),
        None,
    ));
    assert!(!should_replace_render_queue_head(
        &incoming,
        &existing,
        Instant::now(),
        None,
    ));
}

#[test]
fn owner_frame_in_same_epoch_replaces_non_owner_render_candidate() {
    let mut existing = make_decoded_frame_with_epoch(21, Some(8));
    existing.rtp_timestamp = 121;
    existing.recovery_owner_rtp_timestamp = Some(120);
    existing.surface.rtp_timestamp = Some(121);
    existing.surface.recovery_owner_rtp_timestamp = Some(120);

    let mut incoming = make_decoded_frame_with_epoch(20, Some(8));
    incoming.rtp_timestamp = 120;
    incoming.recovery_owner_rtp_timestamp = Some(120);
    incoming.surface.rtp_timestamp = Some(120);
    incoming.surface.recovery_owner_rtp_timestamp = Some(120);

    assert!(should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now(),
        None,
    ));
    assert!(!should_replace_render_queue_head(
        &incoming,
        &existing,
        Instant::now(),
        None,
    ));
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
        release_interval_ms: 17,
        host_frame_age_budget_ms: Some(75.0),
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
    assert!(summary.contains("hostFramePresentEpoch=7"));
    assert!(summary.contains("cadencePhase=starved"));
    assert!(summary.contains("hostFrameAgeBudgetMs=75.0"));
}

#[test]
fn render_queue_prefers_newer_plain_delta_over_old_keyframe_without_recovery_signal() {
    let mut existing = make_decoded_frame(101);
    existing.is_keyframe = true;
    existing.surface.is_keyframe = true;
    let incoming = make_decoded_frame(102);
    assert!(should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now(),
        None,
    ));
    assert!(!should_replace_render_queue_head(
        &incoming,
        &existing,
        Instant::now(),
        None,
    ));
}

#[test]
fn steady_continuation_does_not_replace_recovery_window_continuation_in_same_epoch() {
    let mut existing = make_recovery_window_decoded_frame(301);
    existing.recovery_epoch_tag = Some(12);
    existing.surface.recovery_epoch_tag = Some(12);
    existing.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    existing.surface.frame_recovery_disposition = Some("repairing".to_string());

    let mut incoming = make_decoded_frame_with_epoch(302, Some(12));
    incoming.frame_recovery_disposition = FrameRecoveryDisposition::Steady;
    incoming.surface.frame_recovery_disposition = None;

    assert!(!should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now(),
        None,
    ));
    assert!(should_replace_render_queue_head(
        &incoming,
        &existing,
        Instant::now(),
        None,
    ));
}

#[test]
fn owner_rebuilding_supply_beats_non_owner_candidate_in_same_epoch() {
    let mut existing = make_recovery_window_decoded_frame(321);
    existing.recovery_epoch_tag = Some(14);
    existing.surface.recovery_epoch_tag = Some(14);
    existing.rtp_timestamp = 321;
    existing.recovery_owner_rtp_timestamp = Some(300);
    existing.surface.rtp_timestamp = Some(321);
    existing.surface.recovery_owner_rtp_timestamp = Some(300);
    existing.surface.frame_recovery_disposition = Some("rebuilding-supply".to_string());

    let mut incoming = make_recovery_window_decoded_frame(300);
    incoming.recovery_epoch_tag = Some(14);
    incoming.surface.recovery_epoch_tag = Some(14);
    incoming.rtp_timestamp = 300;
    incoming.recovery_owner_rtp_timestamp = Some(300);
    incoming.surface.rtp_timestamp = Some(300);
    incoming.surface.recovery_owner_rtp_timestamp = Some(300);
    incoming.surface.frame_recovery_disposition = Some("rebuilding-supply".to_string());

    assert!(should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now(),
        None,
    ));
    assert!(!should_replace_render_queue_head(
        &incoming,
        &existing,
        Instant::now(),
        None,
    ));
}

#[test]
fn render_queue_replaces_existing_stale_frame_even_if_priority_is_lower() {
    let mut existing = make_keyframe_decoded_frame(201);
    existing.pts = Instant::now() - Duration::from_millis(20);
    let incoming = make_decoded_frame(202);
    assert!(render_frame_is_stale(&existing, Instant::now(), None));
    assert!(should_replace_render_queue_head(
        &existing,
        &incoming,
        Instant::now(),
        None,
    ));
}

#[test]
fn render_frame_stale_window_tracks_30fps_release_interval() {
    let mut frame = make_decoded_frame(401);
    frame.pts = Instant::now() - Duration::from_millis(30);
    assert!(
        !render_frame_is_stale(&frame, Instant::now(), Some(33)),
        "30fps frame should survive one frame interval"
    );
    frame.pts = Instant::now() - Duration::from_millis(38);
    assert!(
        render_frame_is_stale(&frame, Instant::now(), Some(33)),
        "30fps frame should expire beyond interval+guard"
    );
}

#[test]
fn render_frame_stale_window_tracks_60fps_release_interval() {
    let mut frame = make_decoded_frame(402);
    frame.pts = Instant::now() - Duration::from_millis(15);
    assert!(
        !render_frame_is_stale(&frame, Instant::now(), Some(16)),
        "60fps frame should survive one frame interval"
    );
    frame.pts = Instant::now() - Duration::from_millis(20);
    assert!(
        render_frame_is_stale(&frame, Instant::now(), Some(16)),
        "60fps frame should expire beyond interval+guard"
    );
}

#[test]
fn priming_render_queue_accepts_two_pending_frames() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
        XbxEngineMediaRuntimeStats::default(),
    )));
    let mut render_queue = VecDeque::from([make_decoded_frame(1)]);
    let mut frame_drop_observation_id = 0;
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        display_tick_epoch: 9,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Priming,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };

    enqueue_render_frame(
        &mut render_queue,
        make_decoded_frame(2),
        0,
        &runtime_stats,
        &mut frame_drop_observation_id,
        Some(&host_context),
    );

    assert_eq!(render_queue.len(), 2);
    assert_eq!(
        render_queue
            .iter()
            .map(|frame| frame.surface.frame_seq)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn next_wait_duration_ignores_host_release_gate() {
    let policy = FramePacingPolicy::new(16);
    let mut frame = make_decoded_frame(3);
    frame.pts = Instant::now();
    let wait = next_wait_duration(Some(&frame), &policy, Some(Duration::from_millis(7)));
    assert_eq!(wait, Duration::ZERO);
}

#[test]
fn next_wait_duration_holds_future_frame_until_deadline() {
    let policy = FramePacingPolicy::new(16);
    let mut frame = make_decoded_frame(4);
    frame.pts = Instant::now() + Duration::from_millis(40);

    let wait = next_wait_duration(Some(&frame), &policy, None);

    assert!(wait > Duration::ZERO);
    assert!(wait <= Duration::from_millis(16));
}

#[test]
fn cadence_sleep_guard_override_shortens_sleep_during_priming() {
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
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
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
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
fn host_pacing_context_uses_fast_release_when_submit_age_lags_steady_host() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new({
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_display_interval_ms = Some(16.0);
        stats.video_decode_fps = 22.8;
        stats.video_present_fps = 19.0;
        stats.submit_age_ms = Some(357.0);
        stats.display_age_ms = Some(181.0);
        stats.host_frame_present_epoch = 609;
        stats.host_mailbox_enqueue_count_total = 609;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;
        stats
    })));

    let host_context = resolve_host_pacing_context(&runtime_stats, 16);

    assert_eq!(host_context.release_interval_ms, 16);
}

#[test]
fn host_pacing_context_uses_fast_release_for_high_fps_latency_spike() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new({
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_display_interval_ms = Some(8.0);
        stats.video_decode_fps = 52.2;
        stats.video_present_fps = 55.9;
        stats.submit_age_ms = Some(294.0);
        stats.display_age_ms = Some(82.0);
        stats.host_frame_present_epoch = 4_173;
        stats.host_mailbox_enqueue_count_total = 4_178;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;
        stats
    })));

    let host_context = resolve_host_pacing_context(&runtime_stats, 16);

    assert_eq!(host_context.release_interval_ms, 8);
}

#[test]
fn drive_ready_frames_submits_due_frame_without_host_release_window() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
        XbxEngineMediaRuntimeStats::default(),
    )));
    let mut pacing_queue = VecDeque::from([make_decoded_frame(11)]);
    let mut render_queue = VecDeque::new();
    let mut frame_drop_observation_id = 0;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
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
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut render_backpressure_active,
        |_render_queue| {
            submit_calls += 1;
            PendingRenderSubmitResult::Idle
        },
    );

    assert!(dropped.is_none());
    assert_eq!(submit_calls, 3);
    assert!(pacing_queue.is_empty());
    assert_eq!(render_queue.len(), 1);
}

#[test]
fn drive_ready_frames_holds_future_frame_instead_of_fast_releasing_backlog() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
        XbxEngineMediaRuntimeStats::default(),
    )));
    let mut future_frame = make_decoded_frame(12);
    future_frame.pts = Instant::now() + Duration::from_millis(40);
    let mut pacing_queue = VecDeque::from([future_frame]);
    let mut render_queue = VecDeque::new();
    let mut frame_drop_observation_id = 0;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        display_tick_epoch: 0,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Unknown,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let pacing_policy = FramePacingPolicy::with_dynamic_budget(
        host_context.release_interval_ms,
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
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut render_backpressure_active,
        |_render_queue| {
            submit_calls += 1;
            PendingRenderSubmitResult::Idle
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
    let mut frame_drop_observation_id = 0;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
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
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut render_backpressure_active,
        |render_queue| {
            flush_calls += 1;
            if render_queue.is_empty() {
                return PendingRenderSubmitResult::Idle;
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
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
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
fn recovery_keyframe_bypasses_deadline_drop_until_clean_anchor_can_continue() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
        XbxEngineMediaRuntimeStats::default(),
    )));
    let mut frame = make_keyframe_decoded_frame(41);
    frame.recovery_epoch_tag = Some(3);
    frame.surface.recovery_epoch_tag = Some(3);
    frame.recovery_owner_rtp_timestamp = Some(41);
    frame.surface.recovery_owner_rtp_timestamp = Some(41);
    frame.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    frame.surface.frame_recovery_disposition = Some("repairing".to_string());
    frame.pts = Instant::now() - Duration::from_millis(120);

    let mut pacing_queue = VecDeque::from([frame]);
    let mut render_queue = VecDeque::new();
    let mut frame_drop_observation_id = 0;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        display_tick_epoch: 0,
        present_epoch: 0,
        cadence_phase: HostCadencePhaseHint::Unknown,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let pacing_policy = FramePacingPolicy::with_dynamic_budget(
        host_context.release_interval_ms,
        host_context
            .host_frame_age_budget_ms
            .map(|budget_ms| budget_ms.round() as u64),
        resolve_cadence_sleep_guard_override_ms(&host_context),
        host_context.video_rtt_ms,
        host_context.video_nack_recovery_rtt_ms,
    );
    let mut submitted_seqs = Vec::new();

    let dropped = drive_ready_frames_with_submit(
        &mut pacing_queue,
        &mut render_queue,
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut render_backpressure_active,
        |render_queue| {
            if let Some(frame) = render_queue.pop_front() {
                submitted_seqs.push(frame.surface.frame_seq);
                PendingRenderSubmitResult::Submitted(frame)
            } else {
                PendingRenderSubmitResult::Idle
            }
        },
    );

    assert!(dropped.is_none());
    assert!(pacing_queue.is_empty());
    assert!(render_queue.is_empty());
    assert_eq!(submitted_seqs, vec![41]);
}

#[test]
fn starved_host_no_pending_releases_late_continuation_once() {
    let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
        XbxEngineMediaRuntimeStats::default(),
    )));
    let mut frame = make_decoded_frame(42);
    frame.is_keyframe = false;
    frame.surface.is_keyframe = false;
    frame.clean_anchor_commit_recovery_epoch = None;
    frame.frame_unrecoverable_reason = None;
    frame.pts = Instant::now() - Duration::from_millis(120);

    let mut pacing_queue = VecDeque::from([frame]);
    let mut render_queue = VecDeque::new();
    let mut frame_drop_observation_id = 0;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        display_tick_epoch: 120,
        present_epoch: 118,
        cadence_phase: HostCadencePhaseHint::Starved,
        pressure: HostPacingPressure {
            cadence_phase: HostCadencePhaseHint::Starved,
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: 64,
            host_mailbox_overwrite_count_total: 0,
            host_mailbox_enqueue_count_total: 118,
            present_fps: Some(0.0),
            display_fps: Some(60.0),
        },
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let pacing_policy = FramePacingPolicy::with_dynamic_budget(
        host_context.release_interval_ms,
        host_context
            .host_frame_age_budget_ms
            .map(|budget_ms| budget_ms.round() as u64),
        resolve_cadence_sleep_guard_override_ms(&host_context),
        host_context.video_rtt_ms,
        host_context.video_nack_recovery_rtt_ms,
    );
    let mut submitted_seqs = Vec::new();

    let dropped = drive_ready_frames_with_submit(
        &mut pacing_queue,
        &mut render_queue,
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut render_backpressure_active,
        |render_queue| {
            if let Some(frame) = render_queue.pop_front() {
                submitted_seqs.push(frame.surface.frame_seq);
                PendingRenderSubmitResult::Submitted(frame)
            } else {
                PendingRenderSubmitResult::Idle
            }
        },
    );

    assert!(dropped.is_none());
    assert!(pacing_queue.is_empty());
    assert!(render_queue.is_empty());
    assert_eq!(submitted_seqs, vec![42]);
}

#[test]
fn steady_host_still_drops_late_continuation_by_deadline() {
    let stats = Arc::new(std::sync::Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let runtime_stats = RuntimeStatsSink::new(stats.clone());
    let mut frame = make_decoded_frame(43);
    frame.is_keyframe = false;
    frame.surface.is_keyframe = false;
    frame.clean_anchor_commit_recovery_epoch = None;
    frame.frame_unrecoverable_reason = None;
    frame.pts = Instant::now() - Duration::from_millis(120);

    let mut pacing_queue = VecDeque::from([frame]);
    let mut render_queue = VecDeque::new();
    let mut frame_drop_observation_id = 0;
    let mut last_consumed_host_tick_epoch = None;
    let mut render_backpressure_active = false;
    let host_context = HostPacingContext {
        release_interval_ms: 16,
        host_frame_age_budget_ms: Some(36.0),
        display_tick_epoch: 120,
        present_epoch: 120,
        cadence_phase: HostCadencePhaseHint::Steady,
        pressure: HostPacingPressure::default(),
        video_rtt_ms: None,
        video_nack_recovery_rtt_ms: None,
    };
    let pacing_policy = FramePacingPolicy::with_dynamic_budget(
        host_context.release_interval_ms,
        host_context
            .host_frame_age_budget_ms
            .map(|budget_ms| budget_ms.round() as u64),
        resolve_cadence_sleep_guard_override_ms(&host_context),
        host_context.video_rtt_ms,
        host_context.video_nack_recovery_rtt_ms,
    );
    let mut submitted_seqs = Vec::new();

    let dropped = drive_ready_frames_with_submit(
        &mut pacing_queue,
        &mut render_queue,
        &runtime_stats,
        &mut frame_drop_observation_id,
        &pacing_policy,
        &host_context,
        &mut last_consumed_host_tick_epoch,
        &mut render_backpressure_active,
        |render_queue| {
            if let Some(frame) = render_queue.pop_front() {
                submitted_seqs.push(frame.surface.frame_seq);
                PendingRenderSubmitResult::Submitted(frame)
            } else {
                PendingRenderSubmitResult::Idle
            }
        },
    );

    assert!(dropped.is_none());
    assert!(pacing_queue.is_empty());
    assert!(render_queue.is_empty());
    assert!(submitted_seqs.is_empty());
    let stats = stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_pacer_drop_count_total, 1);
}

#[test]
fn host_cadence_gate_submits_frame_to_renderer_without_extra_wait() {
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
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    ));
    let pacer = PacerActorHandle::new(renderer.clone(), runtime_stats.clone(), 16);

    let mut frame = make_decoded_frame(42);
    frame.pts = Instant::now();
    pacer.submit(frame).expect("submit frame to pacer");

    std::thread::sleep(Duration::from_millis(8));
    let early = render_state
        .lock()
        .expect("render state lock")
        .peek_latest_frame()
        .map(|frame| frame.frame_seq);
    assert_eq!(early, Some(42));
    pacer.stop();
    renderer.stop();
}
