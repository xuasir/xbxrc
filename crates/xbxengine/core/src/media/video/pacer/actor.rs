use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::media::video::render::actor::RendererActorHandle;
use crate::media::video::render::pacer::{
    FramePacingAction, FramePacingPolicy, HostCadencePhaseHint, HostPacingPressure,
    QueueHistoryConfig, QueueHistoryController,
};
use crate::media::video::types::DecodedFrame;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::pipeline::observation::record_pipeline_frame_drop;

pub enum PacerMsg {
    Frame(DecodedFrame),
    Stop,
}

pub struct PacerActorHandle {
    tx: SyncSender<PacerMsg>,
}

const PACING_QUEUE_MAX_FRAMES: usize = 3;
const RENDER_QUEUE_MAX_FRAMES: usize = 1;
const RENDER_QUEUE_RETRY_TIMEOUT_MS: u64 = 4;
const HOST_RELEASE_GATE_ALIGNMENT_SLACK_MS: f64 = 1.5;
const HOST_RELEASE_GATE_STALE_GRACE_MULTIPLIER: f64 = 2.5;
const HOST_PRIMING_REUSE_WAIT_RATIO: u64 = 2;

impl PacerActorHandle {
    pub fn new(
        renderer: Arc<RendererActorHandle>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        refresh_interval_ms: u64,
    ) -> Self {
        let runtime_stats = RuntimeStatsSink::new(runtime_stats);
        let (tx, rx) = mpsc::sync_channel(2);

        thread::Builder::new()
            .name("XbxPacerActor".into())
            .spawn(move || {
                run_pacer_loop(rx, renderer, runtime_stats, refresh_interval_ms);
            })
            .expect("Failed to spawn pacer actor thread");

        Self { tx }
    }

    pub fn submit(&self, frame: DecodedFrame) -> Result<(), TrySendError<PacerMsg>> {
        self.tx.try_send(PacerMsg::Frame(frame))
    }

    pub fn stop(&self) {
        let _ = self.tx.send(PacerMsg::Stop);
    }
}

fn run_pacer_loop(
    rx: Receiver<PacerMsg>,
    renderer: Arc<RendererActorHandle>,
    runtime_stats: RuntimeStatsSink,
    refresh_interval_ms: u64,
) {
    let fallback_refresh_interval_ms = refresh_interval_ms;
    let mut catch_up_mode = false;
    let mut last_consumed_host_tick_epoch = None::<u64>;
    let mut frame_drop_observation_id = 0u64;
    let mut pacing_queue: VecDeque<DecodedFrame> = VecDeque::with_capacity(PACING_QUEUE_MAX_FRAMES);
    let mut render_queue: VecDeque<DecodedFrame> = VecDeque::with_capacity(RENDER_QUEUE_MAX_FRAMES);
    let mut queue_history = QueueHistoryController::new(QueueHistoryConfig::default());
    let mut render_backpressure_active = false;

    loop {
        let host_context =
            resolve_host_pacing_context(&runtime_stats, fallback_refresh_interval_ms);
        let sleep_guard_override_ms = resolve_cadence_sleep_guard_override_ms(&host_context);
        enforce_queue_budget(
            &mut pacing_queue,
            &mut queue_history,
            &runtime_stats,
            &mut frame_drop_observation_id,
            &host_context,
        );
        let pacing_policy = FramePacingPolicy::with_dynamic_budget(
            host_context.refresh_interval_ms,
            host_context
                .host_frame_age_budget_ms
                .map(|budget_ms| budget_ms.round() as u64),
            sleep_guard_override_ms,
        );

        let maybe_msg = if pacing_queue.is_empty() && render_queue.is_empty() {
            match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => None,
            }
        } else {
            let host_release_wait = resolve_host_release_wait_duration(
                &host_context,
                crate::media::video::decode::video_decode::now_ms_f64(),
                last_consumed_host_tick_epoch,
            );
            match rx.recv_timeout(
                next_wait_duration(
                    pacing_queue.front(),
                    &pacing_policy,
                    catch_up_mode,
                    host_release_wait,
                )
                .min(Duration::from_millis(RENDER_QUEUE_RETRY_TIMEOUT_MS)),
            ) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };

        if let Some(msg) = maybe_msg {
            match msg {
                PacerMsg::Frame(frame) => {
                    runtime_stats.update(|stats| {
                        stats.video_pacer_submit_count_total =
                            stats.video_pacer_submit_count_total.saturating_add(1);
                    });
                    pacing_queue.push_back(frame);
                    queue_history.record_depth(pacing_queue.len());
                    enforce_queue_budget(
                        &mut pacing_queue,
                        &mut queue_history,
                        &runtime_stats,
                        &mut frame_drop_observation_id,
                        &host_context,
                    );
                }
                PacerMsg::Stop => break,
            }
        }

        if let Some(dropped_frame) = drive_ready_frames(
            &mut pacing_queue,
            &mut render_queue,
            &mut queue_history,
            &renderer,
            &runtime_stats,
            &mut frame_drop_observation_id,
            &pacing_policy,
            &host_context,
            &mut last_consumed_host_tick_epoch,
            &mut catch_up_mode,
            &mut render_backpressure_active,
        ) {
            record_pipeline_frame_drop(
                &runtime_stats,
                &mut frame_drop_observation_id,
                "pacer",
                "drop",
                Some("rendererDisconnected"),
                crate::media::video::decode::video_decode::now_ms_f64(),
                dropped_frame.surface.width,
                dropped_frame.surface.height,
                false,
                render_queue.len(),
                Some(dropped_frame.rtp_timestamp),
                Some(dropped_frame.surface.frame_seq),
                Some(dropped_frame.frame_recovery_disposition),
                dropped_frame.frame_unrecoverable_reason.as_deref(),
            );
            crate::xbx_log_warn!(
                "[XbxPacerActor] renderer unavailable detail=rendererDisconnected, drop frame"
            );
            break;
        }
    }
}

fn enforce_queue_budget(
    pacing_queue: &mut VecDeque<DecodedFrame>,
    queue_history: &mut QueueHistoryController,
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
    host_context: &HostPacingContext,
) {
    while pacing_queue.len() > PACING_QUEUE_MAX_FRAMES {
        if let Some(dropped_frame) = pacing_queue.pop_front() {
            record_pacer_frame_drop(
                runtime_stats,
                frame_drop_observation_id,
                "queueCap",
                dropped_frame,
                pacing_queue.len(),
            );
        }
    }
    let pressure_decision = queue_history.decide_drop_target(&host_context.pressure);
    while pacing_queue.len() > pressure_decision.drop_target {
        if let Some(dropped_frame) = pacing_queue.pop_front() {
            let detail = if pressure_decision.aggressive {
                "queuePressureAggressive"
            } else {
                "queuePressure"
            };
            record_pacer_frame_drop(
                runtime_stats,
                frame_drop_observation_id,
                detail,
                dropped_frame,
                pacing_queue.len(),
            );
        }
    }
    queue_history.record_depth(pacing_queue.len());
}

fn drive_ready_frames(
    pacing_queue: &mut VecDeque<DecodedFrame>,
    render_queue: &mut VecDeque<DecodedFrame>,
    queue_history: &mut QueueHistoryController,
    renderer: &Arc<RendererActorHandle>,
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
    pacing_policy: &FramePacingPolicy,
    host_context: &HostPacingContext,
    last_consumed_host_tick_epoch: &mut Option<u64>,
    catch_up_mode: &mut bool,
    render_backpressure_active: &mut bool,
) -> Option<DecodedFrame> {
    drive_ready_frames_with_submit(
        pacing_queue,
        render_queue,
        queue_history,
        runtime_stats,
        frame_drop_observation_id,
        pacing_policy,
        host_context,
        last_consumed_host_tick_epoch,
        catch_up_mode,
        render_backpressure_active,
        |render_queue| flush_pending_render_output(render_queue, renderer),
    )
}

fn drive_ready_frames_with_submit<F>(
    pacing_queue: &mut VecDeque<DecodedFrame>,
    render_queue: &mut VecDeque<DecodedFrame>,
    queue_history: &mut QueueHistoryController,
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
    pacing_policy: &FramePacingPolicy,
    host_context: &HostPacingContext,
    last_consumed_host_tick_epoch: &mut Option<u64>,
    catch_up_mode: &mut bool,
    render_backpressure_active: &mut bool,
    mut flush_render_output: F,
) -> Option<DecodedFrame>
where
    F: FnMut(&mut VecDeque<DecodedFrame>) -> PendingRenderSubmitResult,
{
    loop {
        match flush_render_output(render_queue) {
            PendingRenderSubmitResult::Submitted => {
                if render_queue.is_empty() && *render_backpressure_active {
                    runtime_stats.update(|stats| {
                        stats.latest_observation_label =
                            Some("pacerRendererBackpressureCleared".to_string());
                        stats.latest_observation_summary = Some("renderQueueDrained".to_string());
                    });
                    *render_backpressure_active = false;
                }
            }
            PendingRenderSubmitResult::Backpressure => {
                if !*render_backpressure_active {
                    runtime_stats.update(|stats| {
                        stats.latest_observation_label =
                            Some("pacerRendererBackpressure".to_string());
                        stats.latest_observation_summary =
                            Some(format!("pendingRenderQueueDepth={}", render_queue.len()));
                    });
                    *render_backpressure_active = true;
                }
                return None;
            }
            PendingRenderSubmitResult::Disconnected(dropped_frame) => {
                return Some(dropped_frame);
            }
        }

        let Some(frame) = pacing_queue.front() else {
            return None;
        };
        let host_release_wait = resolve_host_release_wait_duration(
            host_context,
            crate::media::video::decode::video_decode::now_ms_f64(),
            *last_consumed_host_tick_epoch,
        );
        let decision =
            pacing_policy.decide(Instant::now(), frame.pts, *catch_up_mode, host_release_wait);
        if decision.enter_catch_up_mode {
            *catch_up_mode = true;
        }
        if decision.exit_catch_up_mode {
            *catch_up_mode = false;
        }
        match decision.action {
            FramePacingAction::Sleep(_) => return None,
            FramePacingAction::Drop => {
                let Some(frame) = pacing_queue.pop_front() else {
                    return None;
                };
                record_pacer_frame_drop(
                    runtime_stats,
                    frame_drop_observation_id,
                    "deadline",
                    frame,
                    pacing_queue.len(),
                );
                queue_history.record_depth(pacing_queue.len());
            }
            FramePacingAction::SubmitNow => {
                let Some(frame) = pacing_queue.pop_front() else {
                    return None;
                };
                queue_history.record_depth(pacing_queue.len());
                enqueue_render_frame(
                    render_queue,
                    frame,
                    runtime_stats,
                    frame_drop_observation_id,
                );
                *last_consumed_host_tick_epoch =
                    next_consumed_host_tick_epoch(host_context, *last_consumed_host_tick_epoch);
                match flush_render_output(render_queue) {
                    PendingRenderSubmitResult::Submitted => {}
                    PendingRenderSubmitResult::Backpressure => {
                        if !*render_backpressure_active {
                            runtime_stats.update(|stats| {
                                stats.latest_observation_label =
                                    Some("pacerRendererBackpressure".to_string());
                                stats.latest_observation_summary =
                                    Some(format!("pendingRenderQueueDepth={}", render_queue.len()));
                            });
                            *render_backpressure_active = true;
                        }
                        return None;
                    }
                    PendingRenderSubmitResult::Disconnected(dropped_frame) => {
                        return Some(dropped_frame);
                    }
                }
            }
        }
    }
}

fn enqueue_render_frame(
    render_queue: &mut VecDeque<DecodedFrame>,
    frame: DecodedFrame,
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
) {
    if render_queue.len() >= RENDER_QUEUE_MAX_FRAMES {
        if let Some(replaced_frame) = render_queue.pop_front() {
            record_pacer_frame_drop(
                runtime_stats,
                frame_drop_observation_id,
                "rendererQueueOverflow",
                replaced_frame,
                render_queue.len(),
            );
        }
    }
    render_queue.push_back(frame);
}

#[derive(Debug)]
enum PendingRenderSubmitResult {
    Submitted,
    Backpressure,
    Disconnected(DecodedFrame),
}

fn flush_pending_render_output(
    render_queue: &mut VecDeque<DecodedFrame>,
    renderer: &Arc<RendererActorHandle>,
) -> PendingRenderSubmitResult {
    flush_pending_render_output_with_submit(render_queue, |frame| match renderer.submit(frame) {
        Ok(_) => PendingRenderSubmitResultWithFrame::Submitted,
        Err(TrySendError::Full(crate::media::video::render::actor::RendererMsg::Frame(frame))) => {
            PendingRenderSubmitResultWithFrame::BackpressureWithFrame(frame)
        }
        Err(TrySendError::Disconnected(
            crate::media::video::render::actor::RendererMsg::Frame(frame),
        )) => PendingRenderSubmitResultWithFrame::Disconnected(frame),
        Err(TrySendError::Full(crate::media::video::render::actor::RendererMsg::Stop))
        | Err(TrySendError::Disconnected(crate::media::video::render::actor::RendererMsg::Stop)) => {
            unreachable!()
        }
    })
}

#[derive(Debug)]
enum PendingRenderSubmitResultWithFrame {
    Submitted,
    BackpressureWithFrame(DecodedFrame),
    Disconnected(DecodedFrame),
}

fn flush_pending_render_output_with_submit<F>(
    render_queue: &mut VecDeque<DecodedFrame>,
    mut submit: F,
) -> PendingRenderSubmitResult
where
    F: FnMut(DecodedFrame) -> PendingRenderSubmitResultWithFrame,
{
    let Some(frame) = render_queue.pop_front() else {
        return PendingRenderSubmitResult::Submitted;
    };
    match submit(frame) {
        PendingRenderSubmitResultWithFrame::Submitted => PendingRenderSubmitResult::Submitted,
        PendingRenderSubmitResultWithFrame::BackpressureWithFrame(frame) => {
            render_queue.push_front(frame);
            PendingRenderSubmitResult::Backpressure
        }
        PendingRenderSubmitResultWithFrame::Disconnected(frame) => {
            PendingRenderSubmitResult::Disconnected(frame)
        }
    }
}

fn next_wait_duration(
    frame: Option<&DecodedFrame>,
    pacing_policy: &FramePacingPolicy,
    catch_up_mode: bool,
    host_release_wait: Option<Duration>,
) -> Duration {
    let Some(frame) = frame else {
        return Duration::from_millis(100);
    };
    match pacing_policy
        .decide(Instant::now(), frame.pts, catch_up_mode, host_release_wait)
        .action
    {
        FramePacingAction::Sleep(duration) => duration,
        FramePacingAction::Drop | FramePacingAction::SubmitNow => Duration::ZERO,
    }
}

fn next_consumed_host_tick_epoch(
    host_context: &HostPacingContext,
    last_consumed_host_tick_epoch: Option<u64>,
) -> Option<u64> {
    if host_context.display_tick_epoch > 0 {
        Some(host_context.display_tick_epoch)
    } else {
        last_consumed_host_tick_epoch
    }
}

#[derive(Clone, Debug)]
struct HostPacingContext {
    refresh_interval_ms: u64,
    host_frame_age_budget_ms: Option<f64>,
    latest_host_present_time_ms: Option<f64>,
    display_tick_epoch: u64,
    present_epoch: u64,
    cadence_phase: HostCadencePhaseHint,
    pressure: HostPacingPressure,
}

fn resolve_host_pacing_context(
    runtime_stats: &RuntimeStatsSink,
    fallback_refresh_interval_ms: u64,
) -> HostPacingContext {
    runtime_stats
        .read(|stats| {
            let refresh_interval_ms = stats
                .host_display_interval_ms
                .map(|interval_ms| interval_ms.round() as u64)
                .filter(|interval_ms| *interval_ms > 0)
                .unwrap_or(fallback_refresh_interval_ms);
            HostPacingContext {
                refresh_interval_ms,
                host_frame_age_budget_ms: stats.host_frame_age_budget_ms,
                latest_host_present_time_ms: stats.latest_video_host_present_time_ms,
                display_tick_epoch: stats.host_display_tick_epoch,
                present_epoch: stats.video_present_epoch,
                cadence_phase: HostCadencePhaseHint::from_stats(
                    stats.host_cadence_phase.as_deref(),
                ),
                pressure: HostPacingPressure {
                    cadence_phase: HostCadencePhaseHint::from_stats(
                        stats.host_cadence_phase.as_deref(),
                    ),
                    no_pending_pressure_level: stats.host_no_pending_pressure_level.clone(),
                    no_pending_streak: stats.host_no_pending_streak,
                    present_overwrite_count_total: stats.video_present_overwrite_count_total,
                    present_submit_count_total: stats.video_present_submit_count_total,
                    present_fps: Some(stats.video_present_fps.max(0.0)),
                    display_fps: Some(1_000.0 / refresh_interval_ms as f64),
                },
            }
        })
        .unwrap_or(HostPacingContext {
            refresh_interval_ms: fallback_refresh_interval_ms,
            host_frame_age_budget_ms: None,
            latest_host_present_time_ms: None,
            display_tick_epoch: 0,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Unknown,
            pressure: HostPacingPressure::default(),
        })
}

fn resolve_host_release_wait_duration(
    host_context: &HostPacingContext,
    now_ms: f64,
    last_consumed_host_tick_epoch: Option<u64>,
) -> Option<Duration> {
    let cadence_signal_active = host_context.display_tick_epoch > 0
        && (host_context.present_epoch > 0
            || host_context.cadence_phase.cadence_signal_active());
    if cadence_signal_active {
        let epoch_open = last_consumed_host_tick_epoch
            .map(|last| host_context.display_tick_epoch > last)
            .unwrap_or(true);
        if epoch_open {
            return None;
        }
        match host_context.cadence_phase {
            HostCadencePhaseHint::Starved => {
                // host 已显式进入 no-pending/starved，相同 tick 内也允许尽快补帧。
                return None;
            }
            HostCadencePhaseHint::Priming if host_context.latest_host_present_time_ms.is_none() => {
                // 首轮 priming 还没有 present 节拍时，禁止同一 tick 连续推进多个 release。
                return Some(Duration::from_millis(
                    host_context
                        .refresh_interval_ms
                        .saturating_div(HOST_PRIMING_REUSE_WAIT_RATIO)
                        .max(1),
                ));
            }
            HostCadencePhaseHint::Idle
            | HostCadencePhaseHint::Priming
            | HostCadencePhaseHint::Steady
            | HostCadencePhaseHint::Unknown => {}
        }
    }
    let latest_host_present_time_ms = host_context.latest_host_present_time_ms?;
    let refresh_interval_ms = host_context.refresh_interval_ms.max(1) as f64;
    let host_present_age_ms = (now_ms - latest_host_present_time_ms).max(0.0);
    if host_present_age_ms > refresh_interval_ms * HOST_RELEASE_GATE_STALE_GRACE_MULTIPLIER {
        return None;
    }
    let next_release_due_ms =
        latest_host_present_time_ms + refresh_interval_ms - HOST_RELEASE_GATE_ALIGNMENT_SLACK_MS;
    if now_ms >= next_release_due_ms {
        return None;
    }
    Some(Duration::from_secs_f64(
        ((next_release_due_ms - now_ms) / 1_000.0).max(0.0),
    ))
}

fn resolve_cadence_sleep_guard_override_ms(host_context: &HostPacingContext) -> Option<u64> {
    if matches!(host_context.cadence_phase, HostCadencePhaseHint::Starved) {
        return Some(0);
    }
    if matches!(host_context.cadence_phase, HostCadencePhaseHint::Priming) {
        return Some(
            host_context
                .refresh_interval_ms
                .saturating_div(HOST_PRIMING_REUSE_WAIT_RATIO)
                .max(1),
        );
    }

    let cadence_lag_ratio = host_context
        .pressure
        .display_fps
        .zip(host_context.pressure.present_fps)
        .and_then(|(display_fps, present_fps)| {
            if display_fps <= 0.0 {
                return None;
            }
            Some(((display_fps - present_fps).max(0.0) / display_fps).clamp(0.0, 1.0))
        })
        .unwrap_or(0.0);

    if cadence_lag_ratio >= 0.55 {
        Some(0)
    } else if cadence_lag_ratio >= 0.25 {
        Some(
            host_context
                .refresh_interval_ms
                .saturating_div(HOST_PRIMING_REUSE_WAIT_RATIO)
                .max(1),
        )
    } else {
        None
    }
}

fn record_pacer_frame_drop(
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
    detail: &'static str,
    dropped_frame: DecodedFrame,
    queue_depth: usize,
) {
    runtime_stats.update(|stats| {
        stats.video_pacer_drop_count_total = stats.video_pacer_drop_count_total.saturating_add(1);
    });
    record_pipeline_frame_drop(
        runtime_stats,
        frame_drop_observation_id,
        "pacer",
        "drop",
        Some(detail),
        crate::media::video::decode::video_decode::now_ms_f64(),
        dropped_frame.surface.width,
        dropped_frame.surface.height,
        false,
        queue_depth,
        Some(dropped_frame.rtp_timestamp),
        Some(dropped_frame.surface.frame_seq),
        Some(dropped_frame.frame_recovery_disposition),
        dropped_frame.frame_unrecoverable_reason.as_deref(),
    );
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::{
        drive_ready_frames_with_submit, flush_pending_render_output_with_submit,
        next_wait_duration, resolve_cadence_sleep_guard_override_ms,
        resolve_host_release_wait_duration, HostPacingContext, PendingRenderSubmitResult,
        PendingRenderSubmitResultWithFrame, HostCadencePhaseHint,
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
                PendingRenderSubmitResultWithFrame::Submitted
            }
        });

        assert_eq!(submit_calls, 1);
        assert!(matches!(first, PendingRenderSubmitResult::Backpressure));
        assert_eq!(render_queue.len(), 1);
        assert_eq!(
            render_queue.front().map(|frame| frame.surface.frame_seq),
            Some(1)
        );

        let second = flush_pending_render_output_with_submit(&mut render_queue, |frame| {
            submit_calls += 1;
            assert_eq!(frame.surface.frame_seq, 1);
            PendingRenderSubmitResultWithFrame::Submitted
        });

        assert_eq!(submit_calls, 2);
        assert!(matches!(second, PendingRenderSubmitResult::Submitted));
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
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(1_000.0),
            display_tick_epoch: 0,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Unknown,
            pressure: HostPacingPressure::default(),
        };
        let wait = resolve_host_release_wait_duration(&context, 1_100.0, None);
        assert!(wait.is_none());
    }

    #[test]
    fn host_release_gate_waits_until_next_host_tick_window() {
        let context = HostPacingContext {
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(1_000.0),
            display_tick_epoch: 0,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Unknown,
            pressure: HostPacingPressure::default(),
        };
        let wait = resolve_host_release_wait_duration(&context, 1_006.0, None)
            .expect("host gate should request a wait");
        assert!(wait > Duration::from_millis(7));
        assert!(wait < Duration::from_millis(10));
    }

    #[test]
    fn host_release_gate_prefers_new_display_tick_epoch_over_time_window() {
        let context = HostPacingContext {
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(1_000.0),
            display_tick_epoch: 9,
            present_epoch: 4,
            cadence_phase: HostCadencePhaseHint::Steady,
            pressure: HostPacingPressure::default(),
        };
        let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(8));
        assert!(wait.is_none());
    }

    #[test]
    fn host_release_gate_blocks_reusing_same_display_tick_epoch_until_fallback_window() {
        let context = HostPacingContext {
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(1_000.0),
            display_tick_epoch: 9,
            present_epoch: 4,
            cadence_phase: HostCadencePhaseHint::Steady,
            pressure: HostPacingPressure::default(),
        };
        let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(9))
            .expect("same tick epoch should still gate release");
        assert!(wait > Duration::from_millis(7));
    }

    #[test]
    fn host_release_gate_blocks_reusing_same_priming_tick_before_first_present() {
        let context = HostPacingContext {
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: None,
            display_tick_epoch: 9,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Priming,
            pressure: HostPacingPressure::default(),
        };
        let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(9))
            .expect("priming should block reusing the same host tick before first present");
        assert_eq!(wait, Duration::from_millis(8));
    }

    #[test]
    fn host_release_gate_releases_same_tick_immediately_when_host_is_starved() {
        let context = HostPacingContext {
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(1_000.0),
            display_tick_epoch: 9,
            present_epoch: 4,
            cadence_phase: HostCadencePhaseHint::Starved,
            pressure: HostPacingPressure::default(),
        };
        let wait = resolve_host_release_wait_duration(&context, 1_006.0, Some(9));
        assert!(wait.is_none());
    }

    #[test]
    fn cadence_sleep_guard_override_shortens_sleep_during_priming() {
        let host_context = HostPacingContext {
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: None,
            display_tick_epoch: 1,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Priming,
            pressure: HostPacingPressure::default(),
        };
        assert_eq!(
            resolve_cadence_sleep_guard_override_ms(&host_context),
            Some(8)
        );
    }

    #[test]
    fn cadence_sleep_guard_override_disables_sleep_when_host_is_starved() {
        let host_context = HostPacingContext {
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(1_000.0),
            display_tick_epoch: 9,
            present_epoch: 4,
            cadence_phase: HostCadencePhaseHint::Starved,
            pressure: HostPacingPressure::default(),
        };
        assert_eq!(resolve_cadence_sleep_guard_override_ms(&host_context), Some(0));
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
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(
                crate::media::video::decode::video_decode::now_ms_f64(),
            ),
            display_tick_epoch: 0,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Unknown,
            pressure: HostPacingPressure::default(),
        };
        let pacing_policy = FramePacingPolicy::with_dynamic_budget(
            host_context.refresh_interval_ms,
            host_context
                .host_frame_age_budget_ms
                .map(|budget_ms| budget_ms.round() as u64),
            resolve_cadence_sleep_guard_override_ms(&host_context),
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
                PendingRenderSubmitResult::Submitted
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
            refresh_interval_ms: 16,
            host_frame_age_budget_ms: Some(36.0),
            latest_host_present_time_ms: Some(
                crate::media::video::decode::video_decode::now_ms_f64() - 64.0,
            ),
            display_tick_epoch: 0,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Unknown,
            pressure: HostPacingPressure::default(),
        };
        let pacing_policy = FramePacingPolicy::with_dynamic_budget(
            host_context.refresh_interval_ms,
            host_context
                .host_frame_age_budget_ms
                .map(|budget_ms| budget_ms.round() as u64),
            resolve_cadence_sleep_guard_override_ms(&host_context),
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
                    return PendingRenderSubmitResult::Submitted;
                }
                let frame = render_queue
                    .pop_front()
                    .expect("render queue should contain frame during flush");
                render_queue.push_front(frame);
                PendingRenderSubmitResult::Backpressure
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
                PendingRenderSubmitResult::Submitted
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
}
