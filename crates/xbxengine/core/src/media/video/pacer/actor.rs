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
const RENDER_QUEUE_STALE_SLACK_DELTA_MS: u64 = 4;
const RENDER_QUEUE_STALE_SLACK_REFERENCE_MS: u64 = 8;
const RENDER_QUEUE_STALE_SLACK_KEYFRAME_MS: u64 = 12;

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
        let now = Instant::now();
        if let Some(existing_frame) = render_queue.front() {
            if !should_replace_render_queue_head(existing_frame, &frame, now) {
                record_pacer_frame_drop(
                    runtime_stats,
                    frame_drop_observation_id,
                    "rendererQueueRejectLowerValue",
                    frame,
                    render_queue.len(),
                );
                return;
            }
        }
        if let Some(replaced_frame) = render_queue.pop_front() {
            let detail = if render_frame_is_stale(&replaced_frame, now) {
                "rendererQueueReplaceStale"
            } else {
                "rendererQueueOverflow"
            };
            record_pacer_frame_drop(
                runtime_stats,
                frame_drop_observation_id,
                detail,
                replaced_frame,
                render_queue.len(),
            );
        }
    }
    render_queue.push_back(frame);
}

fn should_replace_render_queue_head(
    existing_frame: &DecodedFrame,
    incoming_frame: &DecodedFrame,
    now: Instant,
) -> bool {
    if render_frame_is_stale(existing_frame, now) {
        return true;
    }
    let existing_priority = render_frame_priority(existing_frame);
    let incoming_priority = render_frame_priority(incoming_frame);
    if incoming_priority > existing_priority {
        return true;
    }
    if incoming_priority < existing_priority {
        return false;
    }
    incoming_frame.pts >= existing_frame.pts
}

fn render_frame_priority(frame: &DecodedFrame) -> u8 {
    match frame.budget.frame_importance() {
        "keyframe" => 3,
        "reference" => 2,
        _ => 1,
    }
}

fn render_frame_is_stale(frame: &DecodedFrame, now: Instant) -> bool {
    now > frame.pts + render_frame_stale_slack(frame)
}

fn render_frame_stale_slack(frame: &DecodedFrame) -> Duration {
    let millis = match frame.budget.frame_importance() {
        "keyframe" => RENDER_QUEUE_STALE_SLACK_KEYFRAME_MS,
        "reference" => RENDER_QUEUE_STALE_SLACK_REFERENCE_MS,
        _ => RENDER_QUEUE_STALE_SLACK_DELTA_MS,
    };
    Duration::from_millis(millis)
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
        && (host_context.present_epoch > 0 || host_context.cadence_phase.cadence_signal_active());
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
#[path = "actor.test.rs"]
mod tests;
