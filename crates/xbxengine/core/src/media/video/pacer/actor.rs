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
const HOST_SAME_TICK_HIGH_REFRESH_LAG_RATIO_MIN: f64 = 0.55;
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
            render_queue.len(),
            &mut queue_history,
            &runtime_stats,
            &mut frame_drop_observation_id,
            &host_context,
        );
        let pacing_policy = FramePacingPolicy::with_dynamic_budget(
            host_context.release_interval_ms, // 使用release限速间隔（油门上限）
            host_context
                .host_frame_age_budget_ms
                .map(|budget_ms| budget_ms.round() as u64),
            sleep_guard_override_ms,
            host_context.video_rtt_ms,
            host_context.video_nack_recovery_rtt_ms,
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
                    if let Some(frame) = pacing_queue.back() {
                        log_pacer_flow(
                            "decodedFrameReady",
                            frame,
                            pacing_queue.len(),
                            render_queue.len(),
                            Some(&host_context),
                            Some("decodedFrameReady"),
                            Some("enterPacer"),
                            None,
                        );
                    }
                    queue_history.record_depth(pacing_queue.len());
                    enforce_queue_budget(
                        &mut pacing_queue,
                        render_queue.len(),
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
            log_pacer_flow(
                "rendererDisconnected",
                &dropped_frame,
                pacing_queue.len(),
                render_queue.len(),
                Some(&host_context),
                Some("rendererDisconnected"),
                Some("drop"),
                None,
            );
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
    render_queue_depth: usize,
    queue_history: &mut QueueHistoryController,
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
    host_context: &HostPacingContext,
) {
    // 动态调整队列容量：根据 cadence phase 决定上限
    let dynamic_queue_cap = match host_context.cadence_phase {
        HostCadencePhaseHint::Starved => 1, // host 饥饿时激进收紧
        HostCadencePhaseHint::Priming => 2, // priming 阶段适度收紧
        _ => PACING_QUEUE_MAX_FRAMES,       // 其他阶段使用默认值 3
    };

    while pacing_queue.len() > dynamic_queue_cap {
        if let Some(dropped_frame) = pacing_queue.pop_front() {
            let detail = if dynamic_queue_cap < PACING_QUEUE_MAX_FRAMES {
                "queueCapDynamic" // 动态容量触发的丢帧
            } else {
                "queueCap"
            };
            record_pacer_frame_drop(
                runtime_stats,
                frame_drop_observation_id,
                detail,
                dropped_frame,
                pacing_queue.len(),
                render_queue_depth,
                Some(host_context),
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
                render_queue_depth,
                Some(host_context),
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
            PendingRenderSubmitResult::Submitted(frame) => {
                log_pacer_flow(
                    "flushSubmitted",
                    &frame,
                    pacing_queue.len(),
                    render_queue.len(),
                    Some(host_context),
                    Some("submit"),
                    Some("rendererAccepted"),
                    None,
                );
                if render_queue.is_empty() && *render_backpressure_active {
                    runtime_stats.update(|stats| {
                        stats.latest_observation_label =
                            Some("pacerRendererBackpressureCleared".to_string());
                        stats.latest_observation_summary = Some("renderQueueDrained".to_string());
                    });
                    *render_backpressure_active = false;
                }
            }
            PendingRenderSubmitResult::Backpressure(frame) => {
                log_pacer_flow(
                    "flushBackpressure",
                    &frame,
                    pacing_queue.len(),
                    render_queue.len(),
                    Some(host_context),
                    Some("rendererBackpressure"),
                    Some("channelFull"),
                    None,
                );
                if !*render_backpressure_active {
                    runtime_stats.update(|stats| {
                        stats.latest_observation_label =
                            Some("pacerRendererBackpressure".to_string());
                        stats.latest_observation_summary =
                            Some(format_render_backpressure_summary(
                                pacing_queue.len(),
                                render_queue.len(),
                                host_context,
                            ));
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
            FramePacingAction::Sleep(_) => {
                log_pacer_flow(
                    "retain",
                    frame,
                    pacing_queue.len(),
                    render_queue.len(),
                    Some(host_context),
                    Some("retain"),
                    Some(if host_release_wait.is_some() {
                        "hostReleaseGate"
                    } else {
                        "pacingDelay"
                    }),
                    None,
                );
                return None;
            }
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
                    render_queue.len(),
                    Some(host_context),
                );
                queue_history.record_depth(pacing_queue.len());
            }
            FramePacingAction::SubmitNow => {
                let Some(frame) = pacing_queue.pop_front() else {
                    return None;
                };
                queue_history.record_depth(pacing_queue.len());
                log_pacer_flow(
                    "submitNow",
                    &frame,
                    pacing_queue.len(),
                    render_queue.len(),
                    Some(host_context),
                    Some("submit"),
                    Some("releaseReady"),
                    None,
                );
                enqueue_render_frame(
                    render_queue,
                    frame,
                    pacing_queue.len(),
                    runtime_stats,
                    frame_drop_observation_id,
                    Some(host_context),
                );
                *last_consumed_host_tick_epoch =
                    next_consumed_host_tick_epoch(host_context, *last_consumed_host_tick_epoch);
                match flush_render_output(render_queue) {
                    PendingRenderSubmitResult::Submitted(frame) => {
                        log_pacer_flow(
                            "flushSubmitted",
                            &frame,
                            pacing_queue.len(),
                            render_queue.len(),
                            Some(host_context),
                            Some("submit"),
                            Some("rendererAccepted"),
                            None,
                        );
                    }
                    PendingRenderSubmitResult::Backpressure(frame) => {
                        log_pacer_flow(
                            "flushBackpressure",
                            &frame,
                            pacing_queue.len(),
                            render_queue.len(),
                            Some(host_context),
                            Some("rendererBackpressure"),
                            Some("channelFull"),
                            None,
                        );
                        if !*render_backpressure_active {
                            runtime_stats.update(|stats| {
                                stats.latest_observation_label =
                                    Some("pacerRendererBackpressure".to_string());
                                stats.latest_observation_summary =
                                    Some(format_render_backpressure_summary(
                                        pacing_queue.len(),
                                        render_queue.len(),
                                        host_context,
                                    ));
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
    pacing_queue_depth: usize,
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
    host_context: Option<&HostPacingContext>,
) {
    if render_queue.len() >= RENDER_QUEUE_MAX_FRAMES {
        let now = Instant::now();
        if let Some(existing_frame) = render_queue.front() {
            if !should_replace_render_queue_head(existing_frame, &frame, now) {
                log_pacer_flow(
                    "renderQueueReject",
                    &frame,
                    pacing_queue_depth,
                    render_queue.len(),
                    host_context,
                    Some("rendererQueueRejectLowerValue"),
                    Some("rejectIncoming"),
                    existing_frame.surface.frame_seq.into(),
                );
                record_pacer_frame_drop(
                    runtime_stats,
                    frame_drop_observation_id,
                    "rendererQueueRejectLowerValue",
                    frame.clone(),
                    pacing_queue_depth,
                    render_queue.len(),
                    host_context,
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
            log_pacer_flow(
                "renderQueueReplace",
                &frame,
                pacing_queue_depth,
                render_queue.len(),
                host_context,
                Some(detail),
                Some("overwritePendingRender"),
                Some(replaced_frame.surface.frame_seq),
            );
            record_pacer_frame_drop(
                runtime_stats,
                frame_drop_observation_id,
                detail,
                replaced_frame,
                pacing_queue_depth,
                render_queue.len(),
                host_context,
            );
        }
    }
    log_pacer_flow(
        "rendererSubmit",
        &frame,
        pacing_queue_depth,
        render_queue.len(),
        host_context,
        Some("submit"),
        Some("enqueueRenderer"),
        None,
    );
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
    match frame.budget.recovery_value_tier() {
        "anchor" => 3,
        "supply" => 2,
        _ => 1,
    }
}

fn render_frame_is_stale(frame: &DecodedFrame, now: Instant) -> bool {
    now > frame.pts + render_frame_stale_slack(frame)
}

fn render_frame_stale_slack(frame: &DecodedFrame) -> Duration {
    let millis = match frame.budget.recovery_value_tier() {
        "anchor" => RENDER_QUEUE_STALE_SLACK_KEYFRAME_MS,
        "supply" => RENDER_QUEUE_STALE_SLACK_REFERENCE_MS,
        _ => RENDER_QUEUE_STALE_SLACK_DELTA_MS,
    };
    Duration::from_millis(millis)
}

#[derive(Debug)]
enum PendingRenderSubmitResult {
    Submitted(DecodedFrame),
    Backpressure(DecodedFrame),
    Disconnected(DecodedFrame),
}

fn flush_pending_render_output(
    render_queue: &mut VecDeque<DecodedFrame>,
    renderer: &Arc<RendererActorHandle>,
) -> PendingRenderSubmitResult {
    flush_pending_render_output_with_submit(render_queue, |frame| match renderer.submit(frame.clone()) {
        Ok(_) => PendingRenderSubmitResultWithFrame::Submitted(frame),
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
    Submitted(DecodedFrame),
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
        return PendingRenderSubmitResult::Submitted(DecodedFrame {
            pts: Instant::now(),
            rtp_timestamp: 0,
            is_keyframe: false,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
            frame_recovery_disposition:
                crate::media::video::types::FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: crate::media::video::render::renderer::XbxRenderFrame {
                width: 0,
                height: 0,
                frame_seq: 0,
                rendered_at_ms: crate::media::video::decode::video_decode::now_ms_f64(),
                rtp_timestamp: Some(0),
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                pixel_data: crate::XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([]),
                },
            },
        });
    };
    match submit(frame) {
        PendingRenderSubmitResultWithFrame::Submitted(frame) => {
            PendingRenderSubmitResult::Submitted(frame)
        }
        PendingRenderSubmitResultWithFrame::BackpressureWithFrame(frame) => {
            render_queue.push_front(frame.clone());
            PendingRenderSubmitResult::Backpressure(frame)
        }
        PendingRenderSubmitResultWithFrame::Disconnected(frame) => {
            PendingRenderSubmitResult::Disconnected(frame)
        }
    }
}

fn log_pacer_flow(
    event: &str,
    frame: &DecodedFrame,
    pacing_queue_depth: usize,
    render_queue_depth: usize,
    host_context: Option<&HostPacingContext>,
    reason: Option<&str>,
    detail: Option<&str>,
    related_frame_seq: Option<u64>,
) {
    let host_tick_epoch = host_context
        .map(|context| context.display_tick_epoch.to_string())
        .unwrap_or_else(|| "-".to_string());
    let host_present_epoch = host_context
        .map(|context| context.present_epoch.to_string())
        .unwrap_or_else(|| "-".to_string());
    let host_cadence_phase = host_context
        .map(|context| context.cadence_phase.as_str().to_string())
        .unwrap_or_else(|| "-".to_string());
    let reason = reason.unwrap_or("-");
    let detail = detail.unwrap_or("-");
    let related_frame_seq = related_frame_seq
        .map(|seq| seq.to_string())
        .unwrap_or_else(|| "-".to_string());
    crate::xbx_log_warn!(
        "[playback-flow][pacer] event={} reason={} detail={} frameSeq={} rtpTimestamp={} isKeyframe={} observedAtMs={} pacingQueueDepth={} renderQueueDepth={} hostTickEpoch={} hostPresentEpoch={} hostCadencePhase={} relatedFrameSeq={}",
        event,
        reason,
        detail,
        frame.surface.frame_seq,
        frame.surface
            .rtp_timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        frame.surface.is_keyframe,
        frame.surface.rendered_at_ms,
        pacing_queue_depth,
        render_queue_depth,
        host_tick_epoch,
        host_present_epoch,
        host_cadence_phase,
        related_frame_seq,
    );
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
    host_refresh_interval_ms: u64, // 真实host刷新间隔（路况）
    release_interval_ms: u64,      // release限速间隔（油门上限）
    host_frame_age_budget_ms: Option<f64>,
    latest_host_present_time_ms: Option<f64>,
    display_tick_epoch: u64,
    present_epoch: u64,
    cadence_phase: HostCadencePhaseHint,
    pressure: HostPacingPressure,
    video_rtt_ms: Option<f64>,
    video_nack_recovery_rtt_ms: Option<f64>,
}

fn resolve_host_pacing_context(
    runtime_stats: &RuntimeStatsSink,
    fallback_refresh_interval_ms: u64,
) -> HostPacingContext {
    runtime_stats
        .read(|stats| {
            // 获取真实host刷新间隔（路况反馈）
            let host_refresh_interval_ms = stats
                .host_display_interval_ms
                .map(|interval_ms| interval_ms.round() as u64)
                .filter(|interval_ms| *interval_ms > 0)
                .unwrap_or(fallback_refresh_interval_ms);

            // 检测视频流实际帧率，计算帧间隔（油门上限）
            let video_frame_interval_ms = detect_video_frame_interval(stats);

            // release限速间隔：优先使用视频流帧间隔，避免高刷新率屏幕造成过度消费
            // 如果视频流是60fps（16.67ms），即使屏幕是144Hz（6.94ms），也按60fps节拍消费
            let release_interval_ms = video_frame_interval_ms.unwrap_or(host_refresh_interval_ms);

            HostPacingContext {
                host_refresh_interval_ms,
                release_interval_ms,
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
                    display_fps: Some(1_000.0 / host_refresh_interval_ms as f64), // 基于真实host刷新率
                },
                video_rtt_ms: stats.video_rtt_ms,
                video_nack_recovery_rtt_ms: stats.video_nack_recovery_rtt_ms,
            }
        })
        .unwrap_or(HostPacingContext {
            host_refresh_interval_ms: fallback_refresh_interval_ms,
            release_interval_ms: fallback_refresh_interval_ms,
            host_frame_age_budget_ms: None,
            latest_host_present_time_ms: None,
            display_tick_epoch: 0,
            present_epoch: 0,
            cadence_phase: HostCadencePhaseHint::Unknown,
            pressure: HostPacingPressure::default(),
            video_rtt_ms: None,
            video_nack_recovery_rtt_ms: None,
        })
}

/// 检测视频流实际帧间隔（基于inbound帧率或decode帧率）
fn detect_video_frame_interval(
    stats: &crate::api::backend::XbxEngineMediaRuntimeStats,
) -> Option<u64> {
    // 优先使用inbound帧率（更准确反映视频流特性）
    // 注意：字段名是 inbound_video_frame_rate_fps 和 video_decode_fps（Rust结构体）
    let video_fps = if stats.inbound_video_frame_rate_fps > 0.0 {
        stats.inbound_video_frame_rate_fps
    } else if stats.video_decode_fps > 0.0 {
        stats.video_decode_fps
    } else {
        return None;
    };

    // 计算帧间隔（毫秒）
    let frame_interval_ms = (1_000.0 / video_fps).round() as u64;

    // 合理性检查：帧间隔应在 8ms-100ms 之间（对应 10fps-120fps）
    if frame_interval_ms >= 8 && frame_interval_ms <= 100 {
        Some(frame_interval_ms)
    } else {
        None
    }
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
                        .release_interval_ms // 使用release限速间隔
                        .saturating_div(HOST_PRIMING_REUSE_WAIT_RATIO)
                        .max(1),
                ));
            }
            HostCadencePhaseHint::Idle
            | HostCadencePhaseHint::Priming
            | HostCadencePhaseHint::Steady
            | HostCadencePhaseHint::Unknown => {}
        }
        let latest_host_present_time_ms = host_context.latest_host_present_time_ms?;
        let host_present_age_ms = (now_ms - latest_host_present_time_ms).max(0.0);
        if allow_high_refresh_same_tick_reuse(host_context, host_present_age_ms) {
            return None;
        }
    }
    let latest_host_present_time_ms = host_context.latest_host_present_time_ms?;
    let release_interval_ms = host_context.release_interval_ms.max(1) as f64; // 使用release限速间隔
    let host_present_age_ms = (now_ms - latest_host_present_time_ms).max(0.0);
    if host_present_age_ms > release_interval_ms * HOST_RELEASE_GATE_STALE_GRACE_MULTIPLIER {
        return None;
    }
    let next_release_due_ms =
        latest_host_present_time_ms + release_interval_ms - HOST_RELEASE_GATE_ALIGNMENT_SLACK_MS;
    if now_ms >= next_release_due_ms {
        return None;
    }
    Some(Duration::from_secs_f64(
        ((next_release_due_ms - now_ms) / 1_000.0).max(0.0),
    ))
}

fn allow_high_refresh_same_tick_reuse(
    host_context: &HostPacingContext,
    host_present_age_ms: f64,
) -> bool {
    if matches!(
        host_context.cadence_phase,
        HostCadencePhaseHint::Priming | HostCadencePhaseHint::Starved
    ) {
        return false;
    }
    if host_context.present_epoch == 0 {
        return false;
    }
    let host_refresh_interval_ms = host_context.host_refresh_interval_ms.max(1) as f64;
    let release_interval_ms = host_context.release_interval_ms.max(1) as f64;
    if release_interval_ms <= host_refresh_interval_ms + HOST_RELEASE_GATE_ALIGNMENT_SLACK_MS {
        return false;
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
    if cadence_lag_ratio < HOST_SAME_TICK_HIGH_REFRESH_LAG_RATIO_MIN {
        return false;
    }
    host_present_age_ms + HOST_RELEASE_GATE_ALIGNMENT_SLACK_MS >= host_refresh_interval_ms
}

fn resolve_cadence_sleep_guard_override_ms(host_context: &HostPacingContext) -> Option<u64> {
    if matches!(host_context.cadence_phase, HostCadencePhaseHint::Starved) {
        return Some(0);
    }
    if matches!(host_context.cadence_phase, HostCadencePhaseHint::Priming) {
        return Some(
            host_context
                .release_interval_ms // 使用release限速间隔
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
                .release_interval_ms // 使用release限速间隔
                .saturating_div(HOST_PRIMING_REUSE_WAIT_RATIO)
                .max(1),
        )
    } else {
        None
    }
}

fn format_render_backpressure_summary(
    pacing_queue_depth: usize,
    pending_render_queue_depth: usize,
    host_context: &HostPacingContext,
) -> String {
    format!(
        "pacingQueueDepth={} pendingRenderQueueDepth={} hostTickEpoch={} presentEpoch={} cadencePhase={} releaseIntervalMs={} hostFrameAgeBudgetMs={}",
        pacing_queue_depth,
        pending_render_queue_depth,
        host_context.display_tick_epoch,
        host_context.present_epoch,
        host_context.cadence_phase.as_str(),
        host_context.release_interval_ms,
        host_context
            .host_frame_age_budget_ms
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "none".to_string())
    )
}

fn record_pacer_frame_drop(
    runtime_stats: &RuntimeStatsSink,
    frame_drop_observation_id: &mut u64,
    detail: &'static str,
    dropped_frame: DecodedFrame,
    pacing_queue_depth: usize,
    render_queue_depth: usize,
    host_context: Option<&HostPacingContext>,
) {
    runtime_stats.update(|stats| {
        stats.video_pacer_drop_count_total = stats.video_pacer_drop_count_total.saturating_add(1);
    });
    log_pacer_flow(
        "drop",
        &dropped_frame,
        pacing_queue_depth,
        render_queue_depth,
        host_context,
        Some(detail),
        Some("drop"),
        None,
    );
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
        pacing_queue_depth,
        Some(dropped_frame.rtp_timestamp),
        Some(dropped_frame.surface.frame_seq),
        Some(dropped_frame.frame_recovery_disposition),
        dropped_frame.frame_unrecoverable_reason.as_deref(),
    );
}

#[cfg(test)]
#[path = "actor.test.rs"]
mod tests;
