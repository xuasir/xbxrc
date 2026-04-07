use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tokio::sync::Notify;

use crate::media::video::decode::video_decode::XbxVideoDecodeState;
use crate::media::video::pacer::actor::PacerActorHandle;
use crate::media::video::types::EncodedFrame;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::pipeline::observation::record_pipeline_frame_drop;

const DECODER_STALL_PACKET_FRESH_MAX_AGE_MS: f64 = 400.0;
const DECODER_STALL_DECODE_AGE_MS: f64 = 1_000.0;
const DECODE_MAILBOX_CAPACITY: usize = 2;
const DECODE_OUTPUT_QUEUE_CAPACITY: usize = 2;
const PENDING_PACER_RETRY_TIMEOUT_MS: u64 = 4;

pub enum DecodeMsg {
    Frame(EncodedFrame),
    LocalDecoderReset { reason: String, observed_at_ms: f64 },
    Stop,
}

pub enum DecodeSubmitError {
    Full(EncodedFrame),
    Disconnected(EncodedFrame),
}

pub struct DecodeActorHandle {
    tx: SyncSender<DecodeMsg>,
    available_slots: Arc<AtomicUsize>,
    pending_output_backpressure: Arc<AtomicBool>,
    demand_epoch: Arc<AtomicU64>,
    demand_notify: Arc<Notify>,
}

#[derive(Clone, Copy, Debug)]
pub struct DecodeDemandSnapshot {
    pub available_input_slots: usize,
    pub pending_output_backpressure: bool,
    pub accepts_input: bool,
}

impl DecodeActorHandle {
    pub fn new(
        pacer: Arc<PacerActorHandle>,
        runtime_stats: Arc<std::sync::Mutex<crate::XbxEngineMediaRuntimeStats>>,
        min_delay_ms: u64,
        max_delay_ms: u64,
    ) -> Self {
        let runtime_stats = RuntimeStatsSink::new(runtime_stats);
        let (tx, rx) = mpsc::sync_channel(DECODE_MAILBOX_CAPACITY);
        let available_slots = Arc::new(AtomicUsize::new(DECODE_MAILBOX_CAPACITY));
        let pending_output_backpressure = Arc::new(AtomicBool::new(false));
        let demand_epoch = Arc::new(AtomicU64::new(1));
        let demand_notify = Arc::new(Notify::new());
        let available_slots_for_thread = available_slots.clone();
        let pending_output_backpressure_for_thread = pending_output_backpressure.clone();
        let demand_epoch_for_thread = demand_epoch.clone();
        let demand_notify_for_thread = demand_notify.clone();

        thread::Builder::new()
            .name("XbxDecodeActor".into())
            .spawn(move || {
                run_decode_loop(
                    rx,
                    pacer,
                    runtime_stats,
                    min_delay_ms,
                    max_delay_ms,
                    available_slots_for_thread,
                    pending_output_backpressure_for_thread,
                    demand_epoch_for_thread,
                    demand_notify_for_thread,
                );
            })
            .expect("Failed to spawn decode actor thread");

        Self {
            tx,
            available_slots,
            pending_output_backpressure,
            demand_epoch,
            demand_notify,
        }
    }

    pub fn submit(&self, frame: EncodedFrame) -> Result<(), DecodeSubmitError> {
        match self.tx.try_send(DecodeMsg::Frame(frame)) {
            Ok(_) => {
                self.available_slots.fetch_sub(1, Ordering::AcqRel);
                Ok(())
            }
            Err(e) => match e {
                TrySendError::Full(DecodeMsg::Frame(frame)) => Err(DecodeSubmitError::Full(frame)),
                TrySendError::Disconnected(DecodeMsg::Frame(frame)) => {
                    crate::xbx_log_error!(
                        "[DecodeActorHandle] Decode thread is disconnected (likely panicked)!"
                    );
                    Err(DecodeSubmitError::Disconnected(frame))
                }
                TrySendError::Full(_) | TrySendError::Disconnected(_) => unreachable!(),
            },
        }
    }

    pub fn available_slots(&self) -> usize {
        self.available_slots.load(Ordering::Acquire)
    }

    pub fn demand_snapshot(&self) -> DecodeDemandSnapshot {
        let available_input_slots = self.available_slots();
        let pending_output_backpressure = self.pending_output_backpressure.load(Ordering::Acquire);
        DecodeDemandSnapshot {
            available_input_slots,
            pending_output_backpressure,
            accepts_input: available_input_slots > 0 && !pending_output_backpressure,
        }
    }

    pub fn demand_epoch(&self) -> u64 {
        self.demand_epoch.load(Ordering::Acquire)
    }

    pub async fn wait_for_demand_change_since(&self, observed_epoch: u64) -> u64 {
        loop {
            let latest_epoch = self.demand_epoch();
            if latest_epoch != observed_epoch {
                return latest_epoch;
            }
            self.demand_notify.notified().await;
        }
    }

    pub fn stop(&self) {
        let _ = self.tx.send(DecodeMsg::Stop);
    }

    pub fn request_local_decoder_reset(&self, reason: impl Into<String>, observed_at_ms: f64) {
        let reason = reason.into();
        if let Err(err) = self.tx.send(DecodeMsg::LocalDecoderReset {
            reason: reason.clone(),
            observed_at_ms,
        }) {
            crate::xbx_log_warn!(
                "[XbxDecodeActor] local decoder reset request dropped because decode thread disconnected reason={} err={}",
                reason,
                err
            );
        }
    }
}

fn run_decode_loop(
    rx: Receiver<DecodeMsg>,
    pacer: Arc<PacerActorHandle>,
    runtime_stats: RuntimeStatsSink,
    min_delay_ms: u64,
    max_delay_ms: u64,
    available_slots: Arc<AtomicUsize>,
    pending_output_backpressure: Arc<AtomicBool>,
    demand_epoch: Arc<AtomicU64>,
    demand_notify: Arc<Notify>,
) {
    // 设置线程局部的 panic hook，确保崩溃信息能被记录到 xbx_log
    std::panic::set_hook(Box::new(|panic_info| {
        let msg = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            &s[..]
        } else {
            "Box<Any>"
        };
        crate::xbx_log_error!(
            "[XbxDecodeActor] PANIC occurred: {} at {:?}",
            msg,
            panic_info.location()
        );
    }));

    let mut decode_state = match XbxVideoDecodeState::new(min_delay_ms, max_delay_ms) {
        Ok(state) => state,
        Err(e) => {
            crate::xbx_log_error!("Failed to initialize hardware decoder: {:?}", e);
            return;
        }
    };
    let mut recent_decode_times_ms = std::collections::VecDeque::<f64>::new();
    let mut frame_drop_observation_id = 0u64;
    let mut decode_candidate_decision_id = 0u64;
    let mut decode_recovery_transition_id = 0u64;
    let mut input_closed = false;
    let mut pending_output_backpressure_active = false;
    sync_decode_runtime_stats(&runtime_stats, &decode_state, 0.0);

    loop {
        if let Some(dropped_frame) =
            drain_pending_decoded_output(&mut decode_state, &runtime_stats, &pacer)
        {
            record_pipeline_frame_drop(
                &runtime_stats,
                &mut frame_drop_observation_id,
                "decode",
                "drop",
                Some("pacerDisconnected"),
                crate::media::video::decode::video_decode::now_ms_f64(),
                dropped_frame.surface.width,
                dropped_frame.surface.height,
                false,
                decode_state.decoded_frame_queue_len(),
                Some(dropped_frame.rtp_timestamp),
                Some(dropped_frame.surface.frame_seq),
                Some(dropped_frame.frame_recovery_disposition),
                dropped_frame.frame_unrecoverable_reason.as_deref(),
            );
            continue;
        }

        if input_closed && !decode_state.ingress_demand().should_pull_output_first() {
            if pending_output_backpressure_active {
                runtime_stats.update(|stats| {
                    stats.latest_observation_label =
                        Some("decodePacerBackpressureCleared".to_string());
                    stats.latest_observation_summary = Some("pendingOutputDrained".to_string());
                });
                set_pending_output_backpressure(
                    &pending_output_backpressure,
                    false,
                    &demand_epoch,
                    &demand_notify,
                );
            }
            break;
        }

        if decode_state.ingress_demand().should_pull_output_first() {
            if !pending_output_backpressure_active {
                runtime_stats.update(|stats| {
                    stats.latest_observation_label = Some("decodePacerBackpressure".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "pendingOutputQueueDepth={}",
                        decode_state.decoded_frame_queue_len()
                    ));
                });
                pending_output_backpressure_active = true;
                set_pending_output_backpressure(
                    &pending_output_backpressure,
                    true,
                    &demand_epoch,
                    &demand_notify,
                );
            }
            std::thread::sleep(Duration::from_millis(PENDING_PACER_RETRY_TIMEOUT_MS));
            continue;
        }

        if pending_output_backpressure_active {
            runtime_stats.update(|stats| {
                stats.latest_observation_label = Some("decodePacerBackpressureCleared".to_string());
                stats.latest_observation_summary = Some("pendingOutputDrained".to_string());
            });
            pending_output_backpressure_active = false;
            set_pending_output_backpressure(
                &pending_output_backpressure,
                false,
                &demand_epoch,
                &demand_notify,
            );
        }

        let maybe_msg = match rx.recv() {
            Ok(msg) => Some(msg),
            Err(_) => {
                input_closed = true;
                None
            }
        };

        if let Some(msg) = maybe_msg {
            match msg {
                DecodeMsg::Frame(frame) => {
                    release_decode_slot(&available_slots, &demand_epoch, &demand_notify);
                    let now_ms = crate::media::video::decode::video_decode::now_ms_f64();
                    if let Some(dropped_frame) = decode_state.process_encoded_frame(frame, now_ms) {
                        record_pipeline_frame_drop(
                            &runtime_stats,
                            &mut frame_drop_observation_id,
                            "decode",
                            "drop",
                            Some("outputQueueOverflow"),
                            now_ms,
                            dropped_frame.surface.width,
                            dropped_frame.surface.height,
                            false,
                            DECODE_OUTPUT_QUEUE_CAPACITY,
                            Some(dropped_frame.rtp_timestamp),
                            Some(dropped_frame.surface.frame_seq),
                            Some(dropped_frame.frame_recovery_disposition),
                            dropped_frame.frame_unrecoverable_reason.as_deref(),
                        );
                    }
                    if decode_state.last_decode_ok_time_ms() == Some(now_ms) {
                        recent_decode_times_ms.push_back(now_ms);
                        while let Some(front) = recent_decode_times_ms.front().copied() {
                            if now_ms - front <= 1_000.0 {
                                break;
                            }
                            recent_decode_times_ms.pop_front();
                        }
                        runtime_stats.update(|stats| {
                            stats.latest_video_decode_ok_time_ms = Some(now_ms);
                            stats.video_decode_fps = recent_window_fps(&recent_decode_times_ms);
                        });
                    }
                    if let Some(decision) = decode_state.latest_decode_candidate_decision() {
                        if decision.decision_id != decode_candidate_decision_id {
                            decode_candidate_decision_id = decision.decision_id;
                            runtime_stats.update(|stats| {
                                stats.latest_decode_candidate_decision = Some(
                                    crate::api::backend::XbxEnginePipelineCandidateDecisionObservation {
                                        decision_id: decision.decision_id,
                                        state: decision.state.as_str().to_string(),
                                        action: decision.action.to_string(),
                                        detail: decision.detail.to_string(),
                                        frame_seq: decision.frame_seq,
                                        observed_at_ms: decision.observed_at_ms,
                                    },
                                );
                                stats.latest_observation_label =
                                    Some("decodeCandidateState".to_string());
                                stats.latest_observation_summary = Some(format!(
                                    "{}:{}:{}:seq={}",
                                    decision.state.as_str(),
                                    decision.action,
                                    decision.detail,
                                    decision
                                        .frame_seq
                                        .map(|seq| seq.to_string())
                                        .unwrap_or_else(|| "-".to_string())
                                ));
                            });
                        }
                    }
                    if let Some(transition) = decode_state.latest_recovery_transition() {
                        if transition.transition_id != decode_recovery_transition_id {
                            decode_recovery_transition_id = transition.transition_id;
                            runtime_stats.update(|stats| {
                                stats.latest_observation_label =
                                    Some("decodeRecoveryState".to_string());
                                stats.latest_observation_summary = Some(format!(
                                    "{} -> {} via {}{}",
                                    transition.from_state.as_str(),
                                    transition.to_state.as_str(),
                                    transition.event.as_str(),
                                    transition
                                        .frame_seq
                                        .map(|seq| format!(" seq={seq}"))
                                        .unwrap_or_default()
                                ));
                            });
                        }
                    }
                    sync_decode_runtime_stats(&runtime_stats, &decode_state, now_ms);
                }
                DecodeMsg::LocalDecoderReset {
                    reason,
                    observed_at_ms,
                } => {
                    runtime_stats.update(|stats| {
                        stats.latest_observation_label =
                            Some("videoDecoderLocalResetRequested".to_string());
                        stats.latest_observation_summary =
                            Some(format!("reason={reason} observedAtMs={observed_at_ms:.3}"));
                    });
                    if let Err(error) = decode_state.request_local_decoder_reset() {
                        crate::xbx_log_warn!(
                            "[XbxDecodeActor] local decoder reset failed reason={} err={}",
                            reason,
                            error
                        );
                        runtime_stats.update(|stats| {
                            stats.latest_observation_label =
                                Some("videoDecoderLocalResetFailed".to_string());
                            stats.latest_observation_summary =
                                Some(format!("reason={reason} err={error}"));
                        });
                    }
                    sync_decode_runtime_stats(
                        &runtime_stats,
                        &decode_state,
                        crate::media::video::decode::video_decode::now_ms_f64(),
                    );
                }
                DecodeMsg::Stop => {
                    input_closed = true;
                }
            }
        }
    }
}

#[derive(Debug)]
enum PendingDecodedSubmitResult {
    Submitted,
    Backpressure(crate::media::video::types::DecodedFrame),
    Disconnected(crate::media::video::types::DecodedFrame),
}

fn drain_pending_decoded_output(
    decode_state: &mut XbxVideoDecodeState,
    runtime_stats: &RuntimeStatsSink,
    pacer: &Arc<PacerActorHandle>,
) -> Option<crate::media::video::types::DecodedFrame> {
    drain_pending_decoded_output_with_submit(decode_state, runtime_stats, |frame| {
        submit_pending_decoded_output(pacer, frame)
    })
}

fn drain_pending_decoded_output_with_submit<F>(
    decode_state: &mut XbxVideoDecodeState,
    runtime_stats: &RuntimeStatsSink,
    mut submit: F,
) -> Option<crate::media::video::types::DecodedFrame>
where
    F: FnMut(crate::media::video::types::DecodedFrame) -> PendingDecodedSubmitResult,
{
    let now_ms = crate::media::video::decode::video_decode::now_ms_f64();
    while decode_state.has_decoded_frame() {
        let Some(frame) = decode_state.pop_decoded_frame(now_ms) else {
            break;
        };
        if frame.is_keyframe {
            runtime_stats.record_keyframe_request_episode_decoded(
                now_ms,
                frame.rtp_timestamp,
                frame.surface.frame_seq,
            );
        }
        match submit(frame) {
            PendingDecodedSubmitResult::Submitted => {}
            PendingDecodedSubmitResult::Backpressure(frame) => {
                decode_state.requeue_decoded_frame_front(frame);
                return None;
            }
            PendingDecodedSubmitResult::Disconnected(frame) => {
                return Some(frame);
            }
        }
    }
    None
}

fn submit_pending_decoded_output(
    pacer: &Arc<PacerActorHandle>,
    frame: crate::media::video::types::DecodedFrame,
) -> PendingDecodedSubmitResult {
    match pacer.submit(frame) {
        Ok(_) => PendingDecodedSubmitResult::Submitted,
        Err(TrySendError::Full(crate::media::video::pacer::actor::PacerMsg::Frame(frame))) => {
            PendingDecodedSubmitResult::Backpressure(frame)
        }
        Err(TrySendError::Disconnected(crate::media::video::pacer::actor::PacerMsg::Frame(
            frame,
        ))) => PendingDecodedSubmitResult::Disconnected(frame),
        Err(TrySendError::Full(crate::media::video::pacer::actor::PacerMsg::Stop))
        | Err(TrySendError::Disconnected(crate::media::video::pacer::actor::PacerMsg::Stop)) => {
            unreachable!()
        }
    }
}

fn release_decode_slot(
    available_slots: &Arc<AtomicUsize>,
    demand_epoch: &Arc<AtomicU64>,
    demand_notify: &Arc<Notify>,
) {
    let _ = available_slots.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_add(1).min(DECODE_MAILBOX_CAPACITY))
    });
    bump_demand_epoch(demand_epoch, demand_notify);
}

fn set_pending_output_backpressure(
    pending_output_backpressure: &Arc<AtomicBool>,
    active: bool,
    demand_epoch: &Arc<AtomicU64>,
    demand_notify: &Arc<Notify>,
) {
    let previous = pending_output_backpressure.swap(active, Ordering::AcqRel);
    if previous != active {
        bump_demand_epoch(demand_epoch, demand_notify);
    }
}

fn bump_demand_epoch(demand_epoch: &Arc<AtomicU64>, demand_notify: &Arc<Notify>) {
    demand_epoch.fetch_add(1, Ordering::AcqRel);
    demand_notify.notify_waiters();
}

fn recent_window_fps(times: &std::collections::VecDeque<f64>) -> f64 {
    let len = times.len();
    if len < 2 {
        return 0.0;
    }
    let first = times.front().copied().unwrap_or_default();
    let last = times.back().copied().unwrap_or(first);
    let window_ms = (last - first).max(1.0);
    ((len.saturating_sub(1)) as f64 * 1_000.0 / window_ms).max(0.0)
}

fn sync_decode_runtime_stats(
    runtime_stats: &RuntimeStatsSink,
    decode_state: &XbxVideoDecodeState,
    now_ms: f64,
) {
    let video_decoder_stalled = derive_decoder_stalled(runtime_stats, now_ms);
    runtime_stats.update(|stats| {
        stats.video_decoder_backend_name = Some(decode_state.decoder_backend_name().to_string());
        stats.video_decoder_reset_count = decode_state.decoder_reset_count();
        stats.latest_video_decoder_reset_time_ms = decode_state.latest_decoder_reset_time_ms();
        stats.video_decode_output_drop_count_total = decode_state.decoded_frame_drop_count();
        stats.video_decoder_hardware_failure_streak = decode_state.hardware_decode_failure_streak();
        stats.latest_video_decoder_hardware_failure_time_ms =
            decode_state.latest_hardware_decode_failure_time_ms();
        stats.latest_video_decoder_hardware_failure_status =
            decode_state.latest_hardware_decode_failure_status();
        stats.video_decoder_recovery_state =
            Some(decode_state.recovery_state().as_str().to_string());
        stats.video_decoder_recovery_state_changed_at_ms =
            decode_state.latest_recovery_state_change_time_ms();
        stats.video_decoder_recovery_event = decode_state
            .latest_recovery_transition()
            .map(|transition| transition.event.as_str().to_string());
        stats.video_decoder_recovery_detail = decode_state
            .latest_recovery_transition()
            .map(|transition| transition.detail.to_string());
        stats.video_decoder_recovery_status = decode_state
            .latest_recovery_transition()
            .and_then(|transition| transition.status);
        stats.latest_video_decoder_probe_observation =
            decode_state.latest_decoder_probe().map(|probe| {
                crate::XbxEngineVideoDecoderProbeObservation {
                    observation_id: probe.observation_id,
                    selected_backend_name: probe.selected_backend_name.clone(),
                    selected_backend_kind: probe.selected_backend_kind.clone(),
                    fallback_count: probe.fallback_count,
                    fallback_summary: probe.fallback_summary.clone(),
                    observed_at_ms: probe.observed_at_ms,
                }
            });
        stats.video_decoder_stalled = Some(video_decoder_stalled);
    });
}

fn derive_decoder_stalled(runtime_stats: &RuntimeStatsSink, now_ms: f64) -> bool {
    runtime_stats
        .read(|stats| {
            let packet_age_ms = stats
                .latest_video_packet_arrival_time_ms
                .map(|at_ms| (now_ms - at_ms).max(0.0))
                .unwrap_or(f64::INFINITY);
            if packet_age_ms > DECODER_STALL_PACKET_FRESH_MAX_AGE_MS {
                return false;
            }
            let decode_age_ms = stats
                .latest_video_decode_ok_time_ms
                .map(|at_ms| (now_ms - at_ms).max(0.0))
                .unwrap_or(f64::INFINITY);
            decode_age_ms >= DECODER_STALL_DECODE_AGE_MS
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{drain_pending_decoded_output_with_submit, PendingDecodedSubmitResult};
    use crate::media::video::decode::video_decode::XbxVideoDecodeState;
    use crate::media::video::render::renderer::XbxRenderFrame;
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::XbxEngineRenderPixelData;

    fn make_render_frame(frame_seq: u64) -> XbxRenderFrame {
        XbxRenderFrame {
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
        }
    }

    #[test]
    fn pending_decoded_output_keeps_frame_on_backpressure_until_retry_succeeds() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_for_test(make_render_frame(1));
        state.enqueue_decoded_frame_for_test(make_render_frame(2));

        let mut submit_calls = 0usize;
        let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        )));
        let first_pass =
            drain_pending_decoded_output_with_submit(&mut state, &runtime_stats, |frame| {
                submit_calls += 1;
                assert_eq!(frame.surface.frame_seq, submit_calls as u64);
                if submit_calls == 1 {
                    PendingDecodedSubmitResult::Backpressure(frame)
                } else {
                    PendingDecodedSubmitResult::Submitted
                }
            });

        assert!(first_pass.is_none());
        assert_eq!(submit_calls, 1);
        assert!(state.has_decoded_frame());
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );

        let second_pass =
            drain_pending_decoded_output_with_submit(&mut state, &runtime_stats, |frame| {
                submit_calls += 1;
                if submit_calls == 2 {
                    assert_eq!(frame.surface.frame_seq, 1);
                } else {
                    assert_eq!(frame.surface.frame_seq, 2);
                }
                PendingDecodedSubmitResult::Submitted
            });

        assert!(second_pass.is_none());
        assert_eq!(submit_calls, 3);
        assert!(!state.has_decoded_frame());
    }

    #[test]
    fn pending_decoded_output_reports_disconnect_without_silently_requeueing() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_for_test(make_render_frame(2));
        state.enqueue_decoded_frame_for_test(make_render_frame(3));

        let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        )));
        let dropped =
            drain_pending_decoded_output_with_submit(&mut state, &runtime_stats, |frame| {
                assert_eq!(frame.surface.frame_seq, 2);
                PendingDecodedSubmitResult::Disconnected(frame)
            });

        assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(2));
        assert!(state.has_decoded_frame());
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(3)
        );
    }

    #[test]
    fn decoded_output_queue_remains_bounded_when_front_frame_is_requeued() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_for_test(make_render_frame(1));
        state.enqueue_decoded_frame_for_test(make_render_frame(2));

        let frame = state
            .pop_decoded_frame(0.0)
            .expect("front frame should exist");
        state.requeue_decoded_frame_front(frame);

        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert_eq!(state.decoded_frame_queue_len(), 2);
    }
}
