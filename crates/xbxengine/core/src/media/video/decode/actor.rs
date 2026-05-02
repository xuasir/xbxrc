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
const DECODE_OUTPUT_MAILBOX_CAPACITY: usize = 2;
const PENDING_PACER_RETRY_TIMEOUT_MS: u64 = 4;
const STALE_RECOVERY_CONTINUATION_WINDOW_MS: f64 = 80.0;
const STALE_RECOVERY_CONTINUATION_MAX_FRAMES: u32 = 3;

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

    #[cfg(test)]
    pub(crate) fn from_test_sender(tx: SyncSender<DecodeMsg>) -> Self {
        Self {
            tx,
            available_slots: Arc::new(AtomicUsize::new(DECODE_MAILBOX_CAPACITY)),
            pending_output_backpressure: Arc::new(AtomicBool::new(false)),
            demand_epoch: Arc::new(AtomicU64::new(1)),
            demand_notify: Arc::new(Notify::new()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EncodedFrameRecoveryClass {
    recovery_epoch_tag: Option<u64>,
    recovery_owner_rtp_timestamp: Option<u32>,
    is_recovery_anchor: bool,
    is_delta_continuation: bool,
    width: u32,
    height: u32,
    rtp_timestamp: u32,
    is_keyframe: bool,
    frame_recovery_disposition: crate::media::video::types::FrameRecoveryDisposition,
}

impl EncodedFrameRecoveryClass {
    fn from_frame(frame: &EncodedFrame) -> Self {
        Self {
            recovery_epoch_tag: frame.recovery_epoch_tag,
            recovery_owner_rtp_timestamp: frame.recovery_owner_rtp_timestamp,
            is_recovery_anchor: frame.is_keyframe || frame.value.is_sync_point(),
            is_delta_continuation: frame.h264.delta_continuation_ready(),
            width: frame.width,
            height: frame.height,
            rtp_timestamp: frame.rtp_timestamp,
            is_keyframe: frame.is_keyframe,
            frame_recovery_disposition: frame.frame_recovery_disposition,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct StaleRecoveryBridge {
    epoch: Option<u64>,
    deadline_at_ms: Option<f64>,
    frames_left: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreDecodeRecoveryFenceVerdict {
    AllowCurrentOrFuture,
    AllowStaleAnchor,
    AllowStaleContinuation,
    DropStaleFrame { detail: &'static str },
}

impl StaleRecoveryBridge {
    fn evaluate(
        &mut self,
        frame: EncodedFrameRecoveryClass,
        current_epoch: u64,
        now_ms: f64,
    ) -> PreDecodeRecoveryFenceVerdict {
        let Some(frame_epoch) = frame.recovery_epoch_tag else {
            return PreDecodeRecoveryFenceVerdict::AllowCurrentOrFuture;
        };
        if frame_epoch >= current_epoch {
            if frame_epoch > current_epoch {
                self.clear();
            }
            return PreDecodeRecoveryFenceVerdict::AllowCurrentOrFuture;
        }
        if frame.is_recovery_anchor {
            return PreDecodeRecoveryFenceVerdict::AllowStaleAnchor;
        }
        if self.try_consume_continuation(frame_epoch, frame.is_delta_continuation, now_ms) {
            return PreDecodeRecoveryFenceVerdict::AllowStaleContinuation;
        }
        PreDecodeRecoveryFenceVerdict::DropStaleFrame {
            detail: if frame.is_delta_continuation {
                "staleRecoveryContinuationExpired"
            } else {
                "staleRecoveryEpochFrame"
            },
        }
    }

    fn on_decode_success(
        &mut self,
        verdict: PreDecodeRecoveryFenceVerdict,
        frame_epoch: Option<u64>,
        current_epoch: u64,
        now_ms: f64,
    ) {
        match verdict {
            PreDecodeRecoveryFenceVerdict::AllowStaleAnchor => {
                if let Some(epoch) = frame_epoch {
                    self.epoch = Some(epoch);
                    self.deadline_at_ms = Some(now_ms + STALE_RECOVERY_CONTINUATION_WINDOW_MS);
                    self.frames_left = STALE_RECOVERY_CONTINUATION_MAX_FRAMES;
                }
            }
            PreDecodeRecoveryFenceVerdict::AllowCurrentOrFuture => {
                if frame_epoch.is_some_and(|epoch| epoch >= current_epoch) {
                    self.clear();
                }
            }
            PreDecodeRecoveryFenceVerdict::AllowStaleContinuation
            | PreDecodeRecoveryFenceVerdict::DropStaleFrame { .. } => {}
        }
    }

    fn try_consume_continuation(
        &mut self,
        frame_epoch: u64,
        is_delta_continuation: bool,
        now_ms: f64,
    ) -> bool {
        let within_window = self
            .deadline_at_ms
            .is_some_and(|deadline_at_ms| now_ms <= deadline_at_ms);
        if self.epoch != Some(frame_epoch)
            || !is_delta_continuation
            || !within_window
            || self.frames_left == 0
        {
            if self.epoch == Some(frame_epoch) && (!within_window || self.frames_left == 0) {
                self.clear();
            }
            return false;
        }
        self.frames_left = self.frames_left.saturating_sub(1);
        if self.frames_left == 0 {
            self.clear();
        }
        true
    }

    fn clear(&mut self) {
        self.epoch = None;
        self.deadline_at_ms = None;
        self.frames_left = 0;
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
    let mut stale_recovery_bridge = StaleRecoveryBridge::default();
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
                None,
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
                    stats.latest_observation_summary = Some(format_decode_backpressure_summary(
                        decode_state.decoded_frame_queue_len(),
                        decode_state
                            .latest_decode_candidate_decision()
                            .map(|decision| decision.detail),
                        decode_state
                            .latest_recovery_transition()
                            .map(|transition| transition.to_state.as_str()),
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
                    let frame_rtp_timestamp = frame.rtp_timestamp;
                    let frame_class = EncodedFrameRecoveryClass::from_frame(&frame);
                    let current_recovery_epoch = runtime_stats
                        .read(|stats| stats.transport_recovery_epoch)
                        .unwrap_or(0);
                    let fence_verdict =
                        stale_recovery_bridge.evaluate(frame_class, current_recovery_epoch, now_ms);
                    if let PreDecodeRecoveryFenceVerdict::DropStaleFrame { detail } = fence_verdict
                    {
                        record_pipeline_frame_drop(
                            &runtime_stats,
                            &mut frame_drop_observation_id,
                            "decode",
                            "drop",
                            Some(detail),
                            now_ms,
                            frame_class.width,
                            frame_class.height,
                            frame_class.is_keyframe,
                            decode_state.decoded_frame_queue_len(),
                            Some(frame_class.rtp_timestamp),
                            None,
                            Some(frame_class.frame_recovery_disposition),
                            None,
                            None,
                        );
                        runtime_stats.update(|stats| {
                            stats.latest_observation_label =
                                Some("decodeStaleRecoveryFenceDrop".to_string());
                            stats.latest_observation_summary = Some(format!(
                                "detail={} frameEpoch={} currentEpoch={} rtpTimestamp={}",
                                detail,
                                frame_class
                                    .recovery_epoch_tag
                                    .map(|epoch| epoch.to_string())
                                    .unwrap_or_else(|| "-".to_string()),
                                current_recovery_epoch,
                                frame_class.rtp_timestamp,
                            ));
                        });
                        sync_decode_runtime_stats(&runtime_stats, &decode_state, now_ms);
                        continue;
                    }

                    if let Some(dropped_frame) = decode_state.process_encoded_frame(frame, now_ms) {
                        let output_queue_depth = decode_state.decoded_frame_queue_len();
                        record_pipeline_frame_drop(
                            &runtime_stats,
                            &mut frame_drop_observation_id,
                            "decode",
                            "drop",
                            Some("supersededAfterDecode"),
                            now_ms,
                            dropped_frame.surface.width,
                            dropped_frame.surface.height,
                            false,
                            DECODE_OUTPUT_MAILBOX_CAPACITY,
                            Some(dropped_frame.rtp_timestamp),
                            Some(dropped_frame.surface.frame_seq),
                            Some(dropped_frame.frame_recovery_disposition),
                            dropped_frame.frame_unrecoverable_reason.as_deref(),
                            decode_state
                                .latest_decode_candidate_decision()
                                .and_then(|decision| decision.replacement_decision.clone()),
                        );
                        runtime_stats.update(|stats| {
                            stats.latest_observation_label =
                                Some("decodeOutputSupersededAfterDecode".to_string());
                            stats.latest_observation_summary = Some(format!(
                                "outputMailboxDepth={} droppedFrameSeq={} droppedRtpTimestamp={} recoveryState={} candidateDetail={}",
                                output_queue_depth,
                                dropped_frame.surface.frame_seq,
                                dropped_frame.rtp_timestamp,
                                decode_state.recovery_state().as_str(),
                                decode_state
                                    .latest_decode_candidate_decision()
                                    .map(|decision| decision.detail)
                                    .unwrap_or("none"),
                            ));
                        });
                    }
                    if decode_state.last_decode_ok_time_ms() == Some(now_ms) {
                        stale_recovery_bridge.on_decode_success(
                            fence_verdict,
                            frame_class.recovery_epoch_tag,
                            current_recovery_epoch,
                            now_ms,
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
                            stats.latest_video_decode_ok_rtp_timestamp = Some(frame_rtp_timestamp);
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
                                        replacement_decision: decision.replacement_decision.clone(),
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
                    match decode_state.request_local_decoder_reset() {
                        Ok(true) => {}
                        Ok(false) => {
                            runtime_stats.update(|stats| {
                                stats.latest_observation_label =
                                    Some("videoDecoderLocalResetCoalesced".to_string());
                                stats.latest_observation_summary = Some(format!(
                                    "reason={reason} detail=awaitSuccessEdgeOrBarrier"
                                ));
                            });
                        }
                        Err(error) => {
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

impl PendingDecodedSubmitResult {
    fn label(&self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Backpressure(_) => "backpressure",
            Self::Disconnected(_) => "disconnected",
        }
    }
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
        let decoded_owner_matches_latest_episode = if frame.is_keyframe {
            runtime_stats
                .read(|stats| {
                    stats
                        .latest_keyframe_request_episode
                        .as_ref()
                        .filter(|episode| {
                            episode.request_reason.as_deref()
                                == Some("transportAwaitRecoveryAnchor")
                        })
                        .is_none_or(|episode| {
                            episode.response_rtp_timestamp == Some(frame.rtp_timestamp)
                        })
                })
                .unwrap_or(true)
        } else {
            true
        };
        if frame.is_keyframe && decoded_owner_matches_latest_episode {
            runtime_stats.record_picture_recovery_episode_decoded(
                now_ms,
                frame.rtp_timestamp,
                frame.surface.frame_seq,
            );
        }
        let clean_anchor_commit_recovery_epoch = frame.clean_anchor_commit_recovery_epoch;
        let frame_rtp_timestamp = frame.rtp_timestamp;
        let recovery_owner_rtp_timestamp = frame.recovery_owner_rtp_timestamp;
        let frame_seq = frame.surface.frame_seq;
        let submit_result = submit(frame);
        runtime_stats.update(|stats| {
            stats.latest_observation_label = Some("decodePacerSubmit".to_string());
            stats.latest_observation_summary = Some(format!(
                "result={} frameSeq={} rtpTimestamp={} cleanAnchorEpoch={} recoveryOwnerRtp={} pendingOutputQueueDepth={} candidateDetail={}",
                submit_result.label(),
                frame_seq,
                frame_rtp_timestamp,
                clean_anchor_commit_recovery_epoch
                    .map(|epoch| epoch.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                recovery_owner_rtp_timestamp
                    .map(|rtp| rtp.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                decode_state.decoded_frame_queue_len(),
                decode_state
                    .latest_decode_candidate_decision()
                    .map(|decision| decision.detail)
                    .unwrap_or("none"),
            ));
        });
        crate::xbx_log_warn!(
            "[playback-flow][decode] event=pacerSubmit result={} frameSeq={} rtpTimestamp={} cleanAnchorEpoch={} recoveryOwnerRtp={} pendingOutputQueueDepth={}",
            submit_result.label(),
            frame_seq,
            frame_rtp_timestamp,
            clean_anchor_commit_recovery_epoch
                .map(|epoch| epoch.to_string())
                .unwrap_or_else(|| "-".to_string()),
            recovery_owner_rtp_timestamp
                .map(|rtp| rtp.to_string())
                .unwrap_or_else(|| "-".to_string()),
            decode_state.decoded_frame_queue_len(),
        );
        match submit_result {
            PendingDecodedSubmitResult::Submitted => {
                if let Some(recovery_epoch) = clean_anchor_commit_recovery_epoch {
                    let (
                        transport_recovery_epoch,
                        clean_anchor_epoch,
                        submission_episode_id,
                        owner_accepts_submission,
                    ) = runtime_stats
                        .read(|stats| {
                            let owner_rtp_timestamp =
                                recovery_owner_rtp_timestamp.or(Some(frame_rtp_timestamp));
                            let current_owner_episode = stats
                                .latest_keyframe_request_episode
                                .as_ref()
                                .filter(|episode| {
                                    episode.request_reason.as_deref()
                                        == Some("transportAwaitRecoveryAnchor")
                                });
                            let inspection_bound_episode_id = stats
                                .latest_h264_inspection_observation
                                .as_ref()
                                .and_then(|inspection| {
                                    (inspection.frame_rtp_timestamp == Some(frame_rtp_timestamp)
                                        && inspection.admission_accepted
                                        && inspection.bound_recovery_epoch
                                            == Some(stats.transport_recovery_epoch))
                                    .then_some(inspection.bound_episode_id)
                                    .flatten()
                                });
                            let bound_episode_id = current_owner_episode
                                .filter(|episode| {
                                    owner_rtp_timestamp.is_some_and(|owner_rtp_timestamp| {
                                        episode.response_rtp_timestamp == Some(owner_rtp_timestamp)
                                            || episode.first_video_packet_rtp_timestamp
                                                == Some(owner_rtp_timestamp)
                                    }) || episode.response_rtp_timestamp
                                        == Some(frame_rtp_timestamp)
                                        || episode.first_video_packet_rtp_timestamp
                                            == Some(frame_rtp_timestamp)
                                })
                                .map(|episode| episode.episode_id)
                                .or_else(|| {
                                    stats.recent_keyframe_request_episodes.iter().find_map(
                                        |episode| {
                                            (episode.request_reason.as_deref()
                                                == Some("transportAwaitRecoveryAnchor")
                                                && (owner_rtp_timestamp.is_some_and(
                                                    |owner_rtp_timestamp| {
                                                        episode.response_rtp_timestamp
                                                            == Some(owner_rtp_timestamp)
                                                            || episode
                                                                .first_video_packet_rtp_timestamp
                                                                == Some(owner_rtp_timestamp)
                                                    },
                                                ) || episode.response_rtp_timestamp
                                                    == Some(frame_rtp_timestamp)
                                                    || episode.first_video_packet_rtp_timestamp
                                                        == Some(frame_rtp_timestamp)))
                                            .then_some(episode.episode_id)
                                        },
                                    )
                                });
                            let current_owner_episode_id =
                                current_owner_episode.map(|episode| episode.episode_id);
                            (
                                stats.transport_recovery_epoch,
                                stats.video_anchor_clean_epoch,
                                bound_episode_id
                                    .or(inspection_bound_episode_id)
                                    .or(current_owner_episode_id),
                                current_owner_episode_id.is_none()
                                    || current_owner_episode_id
                                        == bound_episode_id.or(inspection_bound_episode_id),
                            )
                        })
                        .unwrap_or((0, None, None, false));
                    let should_commit_clean_anchor = transport_recovery_epoch == recovery_epoch
                        && clean_anchor_epoch != Some(recovery_epoch)
                        && submission_episode_id.is_some()
                        && owner_accepts_submission;
                    if should_commit_clean_anchor {
                        runtime_stats.record_anchor_candidate_ledger(
                            recovery_epoch,
                            Some(frame_rtp_timestamp),
                            crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
                            "chain-clean-anchor-submitted",
                            None,
                            now_ms,
                        );
                        runtime_stats.record_transport_clean_anchor_submission(
                            recovery_epoch,
                            submission_episode_id.unwrap_or_default(),
                            frame_rtp_timestamp,
                            now_ms,
                            "chain-clean-anchor-submitted",
                        );
                    } else if transport_recovery_epoch == recovery_epoch
                        && clean_anchor_epoch != Some(recovery_epoch)
                        && submission_episode_id.is_none()
                    {
                        runtime_stats.record_picture_recovery_blocker(
                            now_ms,
                            "media",
                            "cleanAnchorEpisodeUnbound",
                            "warning",
                            Some(frame_rtp_timestamp),
                            Some(frame_seq),
                        );
                    } else if transport_recovery_epoch == recovery_epoch
                        && clean_anchor_epoch != Some(recovery_epoch)
                        && !owner_accepts_submission
                    {
                        runtime_stats.record_picture_recovery_blocker(
                            now_ms,
                            "media",
                            "cleanAnchorOwnerAdvanced",
                            "warning",
                            Some(frame_rtp_timestamp),
                            Some(frame_seq),
                        );
                    } else if transport_recovery_epoch != recovery_epoch
                        && clean_anchor_epoch != Some(recovery_epoch)
                    {
                        runtime_stats.record_picture_recovery_blocker(
                            now_ms,
                            "media",
                            "cleanAnchorCommitEpochAdvanced",
                            "warning",
                            Some(frame_rtp_timestamp),
                            Some(frame_seq),
                        );
                    }
                }
            }
            PendingDecodedSubmitResult::Backpressure(frame) => {
                if clean_anchor_commit_recovery_epoch.is_some() {
                    runtime_stats.record_picture_recovery_blocker(
                        now_ms,
                        "media",
                        "cleanAnchorSubmitBackpressure",
                        "warning",
                        Some(frame_rtp_timestamp),
                        Some(frame.surface.frame_seq),
                    );
                }
                decode_state.requeue_decoded_frame_front(frame);
                return None;
            }
            PendingDecodedSubmitResult::Disconnected(frame) => {
                if clean_anchor_commit_recovery_epoch.is_some() {
                    runtime_stats.record_picture_recovery_blocker(
                        now_ms,
                        "media",
                        "cleanAnchorSubmitDisconnected",
                        "critical",
                        Some(frame_rtp_timestamp),
                        Some(frame.surface.frame_seq),
                    );
                }
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

fn format_decode_backpressure_summary(
    pending_output_queue_depth: usize,
    candidate_detail: Option<&str>,
    recovery_state: Option<&str>,
) -> String {
    format!(
        "pendingOutputQueueDepth={} candidateDetail={} recoveryState={}",
        pending_output_queue_depth,
        candidate_detail.unwrap_or("none"),
        recovery_state.unwrap_or("none")
    )
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
        stats.latest_video_decoder_bootstrap_gate_observation = decode_state
            .latest_bootstrap_gate_observation()
            .map(
                |observation| crate::XbxEngineVideoDecoderBootstrapGateObservation {
                    observation_id: observation.observation_id,
                    recovery_state: observation.recovery_state.as_str().to_string(),
                    frame_rtp_timestamp: observation.frame_rtp_timestamp,
                    is_idr: observation.is_idr,
                    has_inband_sps: observation.has_inband_sps,
                    has_inband_pps: observation.has_inband_pps,
                    committed_sps_present: observation.committed_sps_present,
                    committed_pps_present: observation.committed_pps_present,
                    bootstrap_ready: observation.bootstrap_ready,
                    bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                    observed_at_ms: observation.observed_at_ms,
                },
            );
        stats.latest_decode_output_path_observation = decode_state
            .latest_decode_output_path_observation()
            .map(|observation| crate::XbxEngineDecodeOutputPathObservation {
                observation_id: observation.observation_id,
                verdict: observation.verdict.as_str().to_string(),
                detail: observation.detail.to_string(),
                frame_rtp_timestamp: observation.frame_rtp_timestamp,
                is_keyframe: observation.is_keyframe,
                status: observation.status,
                send_packet_status: observation.send_packet_status,
                receive_frame_status: observation.receive_frame_status,
                backend_no_output_streak: observation.backend_no_output_streak,
                input_frames_since_last_decoded: observation.input_frames_since_last_decoded,
                bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                observed_at_ms: observation.observed_at_ms,
            });
        stats.latest_remote_frame_capture_observation = decode_state
            .latest_remote_frame_capture_observation()
            .map(
                |observation| crate::XbxEngineRemoteFrameCaptureObservation {
                    observation_id: observation.observation_id,
                    trigger: observation.trigger.to_string(),
                    backend_name: observation.backend_name.clone(),
                    frame_rtp_timestamp: observation.frame_rtp_timestamp,
                    is_keyframe: observation.is_keyframe,
                    width: observation.width,
                    height: observation.height,
                    payload_bytes: observation.payload_bytes,
                    payload_fingerprint: observation.payload_fingerprint,
                    payload_prefix_hex: observation.payload_prefix_hex.clone(),
                    nal_types: observation.nal_types.clone(),
                    nal_count: observation.nal_count,
                    has_inband_sps: observation.has_inband_sps,
                    has_inband_pps: observation.has_inband_pps,
                    bootstrap_ready: observation.bootstrap_ready,
                    bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                    parameter_sets_changed: observation.parameter_sets_changed,
                    config_changed: observation.config_changed,
                    slice_headers_valid: observation.slice_headers_valid,
                    send_packet_status: observation.send_packet_status,
                    receive_frame_status: observation.receive_frame_status,
                    status: observation.status,
                    backend_no_output_streak: observation.backend_no_output_streak,
                    input_frames_since_last_decoded: observation.input_frames_since_last_decoded,
                    observed_at_ms: observation.observed_at_ms,
                },
            );
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
    use std::time::Instant;

    use super::{
        drain_pending_decoded_output_with_submit, format_decode_backpressure_summary,
        EncodedFrameRecoveryClass, PendingDecodedSubmitResult, PreDecodeRecoveryFenceVerdict,
        StaleRecoveryBridge,
    };
    use crate::media::video::decode::video_decode::XbxVideoDecodeState;
    use crate::media::video::render::renderer::XbxRenderFrame;
    use crate::media::video::types::{EncodedFrame, FrameValue, VideoCodec};
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::{XbxEngineAnchorCandidateState, XbxEngineRenderPixelData};

    fn make_render_frame(frame_seq: u64) -> XbxRenderFrame {
        XbxRenderFrame {
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
        }
    }

    fn make_encoded_frame(
        recovery_epoch_tag: Option<u64>,
        is_keyframe: bool,
        continuation_ready: bool,
    ) -> EncodedFrame {
        let mut h264 = crate::media::video::test_fixtures::make_bootstrap_assembled_frame(7)
            .into_encoded_frame(Instant::now());
        if continuation_ready {
            h264.h264.commit();
        }
        h264.is_keyframe = is_keyframe;
        h264.value = FrameValue::new(is_keyframe, false, 128);
        h264.rtp_timestamp = 7;
        h264.recovery_epoch_tag = recovery_epoch_tag;
        h264.h264.is_idr = is_keyframe;
        h264.h264.bootstrap_ready = is_keyframe;
        h264.h264.bootstrap_reject_reason = if is_keyframe {
            None
        } else {
            Some(crate::media::video::h264::inspection::H264BootstrapRejectReason::NonIdrVcl)
        };
        h264.codec = VideoCodec::H264;
        h264
    }

    #[test]
    fn stale_recovery_bridge_drops_old_delta_without_anchor() {
        let mut bridge = StaleRecoveryBridge::default();
        let frame =
            EncodedFrameRecoveryClass::from_frame(&make_encoded_frame(Some(2), false, true));

        let verdict = bridge.evaluate(frame, 3, 100.0);

        assert_eq!(
            verdict,
            PreDecodeRecoveryFenceVerdict::DropStaleFrame {
                detail: "staleRecoveryContinuationExpired",
            }
        );
    }

    #[test]
    fn stale_recovery_bridge_allows_short_continuation_after_stale_anchor_decode() {
        let mut bridge = StaleRecoveryBridge::default();
        let stale_anchor =
            EncodedFrameRecoveryClass::from_frame(&make_encoded_frame(Some(2), true, false));
        let stale_delta =
            EncodedFrameRecoveryClass::from_frame(&make_encoded_frame(Some(2), false, true));

        let anchor_verdict = bridge.evaluate(stale_anchor, 3, 100.0);
        assert_eq!(
            anchor_verdict,
            PreDecodeRecoveryFenceVerdict::AllowStaleAnchor
        );
        bridge.on_decode_success(anchor_verdict, Some(2), 3, 100.0);

        assert_eq!(
            bridge.evaluate(stale_delta, 3, 120.0),
            PreDecodeRecoveryFenceVerdict::AllowStaleContinuation
        );
        assert_eq!(
            bridge.evaluate(stale_delta, 3, 130.0),
            PreDecodeRecoveryFenceVerdict::AllowStaleContinuation
        );
        assert_eq!(
            bridge.evaluate(stale_delta, 3, 140.0),
            PreDecodeRecoveryFenceVerdict::AllowStaleContinuation
        );
        assert_eq!(
            bridge.evaluate(stale_delta, 3, 150.0),
            PreDecodeRecoveryFenceVerdict::DropStaleFrame {
                detail: "staleRecoveryContinuationExpired",
            }
        );
    }

    #[test]
    fn current_epoch_decode_success_clears_stale_recovery_bridge() {
        let mut bridge = StaleRecoveryBridge::default();
        let stale_anchor =
            EncodedFrameRecoveryClass::from_frame(&make_encoded_frame(Some(2), true, false));
        let current_anchor =
            EncodedFrameRecoveryClass::from_frame(&make_encoded_frame(Some(3), true, false));
        let stale_delta =
            EncodedFrameRecoveryClass::from_frame(&make_encoded_frame(Some(2), false, true));

        let stale_verdict = bridge.evaluate(stale_anchor, 3, 100.0);
        bridge.on_decode_success(stale_verdict, Some(2), 3, 100.0);
        assert_eq!(
            bridge.evaluate(stale_delta, 3, 110.0),
            PreDecodeRecoveryFenceVerdict::AllowStaleContinuation
        );

        let current_verdict = bridge.evaluate(current_anchor, 3, 120.0);
        assert_eq!(
            current_verdict,
            PreDecodeRecoveryFenceVerdict::AllowCurrentOrFuture
        );
        bridge.on_decode_success(current_verdict, Some(3), 3, 120.0);

        assert_eq!(
            bridge.evaluate(stale_delta, 3, 130.0),
            PreDecodeRecoveryFenceVerdict::DropStaleFrame {
                detail: "staleRecoveryContinuationExpired",
            }
        );
    }

    #[test]
    fn pending_decoded_output_keeps_frame_on_backpressure_until_retry_succeeds() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        // mailbox: 先让 frame=1 进入 inflight，然后再放入 latest candidate=2
        state.enqueue_decoded_frame_for_test(make_render_frame(1));
        let inflight = state.pop_decoded_frame(0.0).expect("inflight should exist");
        state.requeue_decoded_frame_front(inflight);
        state.enqueue_decoded_frame_for_test(make_render_frame(2));

        let mut submit_calls = 0usize;
        let runtime_stats = RuntimeStatsSink::new(Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        )));
        let first_pass =
            drain_pending_decoded_output_with_submit(&mut state, &runtime_stats, |frame| {
                submit_calls += 1;
                // 第一次必须先尝试提交 inflight=1
                if submit_calls == 1 {
                    assert_eq!(frame.surface.frame_seq, 1);
                }
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
        // mailbox: 先让 2 进入 inflight，然后再放入 latest=3
        state.enqueue_decoded_frame_for_test(make_render_frame(2));
        let inflight = state.pop_decoded_frame(0.0).expect("inflight should exist");
        state.requeue_decoded_frame_front(inflight);
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
        // mailbox: 先让 frame=1 占用 inflight，再放一个 latest=2
        state.enqueue_decoded_frame_for_test(make_render_frame(1));
        let inflight = state
            .pop_decoded_frame(0.0)
            .expect("front frame should exist");
        state.requeue_decoded_frame_front(inflight);
        state.enqueue_decoded_frame_for_test(make_render_frame(2));

        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert_eq!(state.decoded_frame_queue_len(), 2);
    }

    #[test]
    fn decoded_keyframe_commits_clean_anchor_only_after_decode() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(1), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(1),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(180.0, 1, 55);

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(stats.latest_clean_anchor_submission_rtp_timestamp, Some(1));
        let ledger = stats
            .latest_anchor_candidate_ledger
            .as_ref()
            .expect("submitted clean-anchor ledger");
        assert_eq!(ledger.recovery_epoch, 1);
        assert_eq!(ledger.frame_rtp_timestamp, Some(1));
        assert_eq!(
            ledger.state,
            XbxEngineAnchorCandidateState::SubmittedCleanAnchor
        );
    }

    #[test]
    fn decode_backpressure_summary_includes_candidate_and_recovery_state() {
        let summary = format_decode_backpressure_summary(
            2,
            Some("supersededAfterDecode"),
            Some("recovering"),
        );

        assert!(summary.contains("pendingOutputQueueDepth=2"));
        assert!(summary.contains("candidateDetail=supersededAfterDecode"));
        assert!(summary.contains("recoveryState=recovering"));
    }

    #[test]
    fn decode_pacer_submit_observation_records_backpressure_and_submit() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_for_test(make_render_frame(7));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        let first = drain_pending_decoded_output_with_submit(&mut state, &sink, |frame| {
            PendingDecodedSubmitResult::Backpressure(frame)
        });
        assert!(first.is_none());
        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            assert_eq!(
                stats.latest_observation_label.as_deref(),
                Some("decodePacerSubmit")
            );
            let summary = stats
                .latest_observation_summary
                .as_deref()
                .expect("backpressure summary");
            assert!(summary.contains("result=backpressure"));
            assert!(summary.contains("frameSeq=7"));
        }

        let second = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });
        assert!(second.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("decodePacerSubmit")
        );
        let summary = stats
            .latest_observation_summary
            .as_deref()
            .expect("submit summary");
        assert!(summary.contains("result=submitted"));
        assert!(summary.contains("frameSeq=7"));
    }

    #[test]
    fn clean_anchor_commit_waits_until_submit_succeeds_after_backpressure() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(1), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(1),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(180.0, 1, 55);

        let first = drain_pending_decoded_output_with_submit(&mut state, &sink, |frame| {
            PendingDecodedSubmitResult::Backpressure(frame)
        });
        assert!(first.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert!(stats.latest_anchor_candidate_ledger.is_none());
        drop(stats);

        let second = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });
        assert!(second.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(stats.latest_clean_anchor_submission_rtp_timestamp, Some(1));
    }

    #[test]
    fn clean_anchor_commit_does_not_satisfy_new_epoch_after_epoch_advances() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(1), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.advance_transport_recovery_episode(20.0);

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, 2);
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, None);
        let blocker = stats
            .latest_picture_recovery_blocker_observation
            .as_ref()
            .expect("epoch advanced blocker");
        assert_eq!(blocker.blocker_kind, "cleanAnchorCommitEpochAdvanced");
        assert_eq!(blocker.gate, "media");
    }

    #[test]
    fn clean_anchor_commit_is_not_recorded_when_submit_disconnects() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(1), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);

        let dropped = drain_pending_decoded_output_with_submit(&mut state, &sink, |frame| {
            PendingDecodedSubmitResult::Disconnected(frame)
        });
        assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(1));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert!(stats.latest_anchor_candidate_ledger.is_none());
    }

    #[test]
    fn clean_anchor_commit_survives_stale_after_decode_drop() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_and_pts_for_test(
            make_render_frame(1),
            Some(1),
            // 确保 stale：让 clean-anchor 候选在 decode stage 被丢弃，不应触发提交。
            std::time::Instant::now() - std::time::Duration::from_millis(200),
        );

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(1),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(180.0, 1, 55);

        let first = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });
        assert!(first.is_none());
        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            assert_eq!(stats.video_anchor_clean_epoch, None);
            assert!(stats.latest_anchor_candidate_ledger.is_none());
        }

        let mut continuation = make_render_frame(2);
        continuation.recovery_owner_rtp_timestamp = Some(1);
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(continuation, Some(1));
        let second = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });
        assert!(second.is_none());

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(stats.latest_clean_anchor_submission_rtp_timestamp, Some(2));
        assert_eq!(
            stats.latest_clean_anchor_submission_source_event.as_deref(),
            Some("chain-clean-anchor-submitted")
        );
        let ledger = stats
            .latest_anchor_candidate_ledger
            .as_ref()
            .expect("submitted clean-anchor ledger");
        assert_eq!(ledger.frame_rtp_timestamp, Some(2));
        assert_eq!(
            ledger.state,
            XbxEngineAnchorCandidateState::SubmittedCleanAnchor
        );
    }

    #[test]
    fn clean_anchor_epoch_advance_records_blocker_without_committing() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(1), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.transport_recovery_epoch = 2;
        }

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, 2);
        assert_eq!(stats.video_anchor_clean_epoch, None);
        let blocker = stats
            .latest_picture_recovery_blocker_observation
            .as_ref()
            .expect("epoch advanced blocker");
        assert_eq!(blocker.blocker_kind, "cleanAnchorCommitEpochAdvanced");
        assert_eq!(blocker.gate, "media");
    }

    #[test]
    fn stale_owner_frame_does_not_submit_clean_anchor_before_new_episode_binds_response() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(1), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(1),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.update(|stats| {
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 1002,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    status: "waiting-response".to_string(),
                    requested_at_ms: 160.0,
                    ..Default::default()
                });
        });

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, None);
        let blocker = stats
            .latest_picture_recovery_blocker_observation
            .as_ref()
            .expect("owner advanced blocker");
        assert_eq!(blocker.blocker_kind, "cleanAnchorOwnerAdvanced");
        assert_eq!(blocker.frame_rtp_timestamp, Some(1));
    }

    #[test]
    fn same_episode_owner_advance_keeps_clean_anchor_submission_pending() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(1), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(1),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_response_observed(
            180.0,
            Some(101),
            true,
            "ownerFrameAdvanced",
            Some(12),
            None,
            false,
            false,
        );

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(stats.latest_clean_anchor_submission_episode_id, Some(1001));
        assert_eq!(stats.latest_clean_anchor_submission_rtp_timestamp, Some(1));
        let transition = stats
            .latest_picture_recovery_transition_observation
            .as_ref()
            .expect("clean anchor submission transition");
        assert_eq!(transition.phase, "CleanAnchorSubmitted");
        assert_eq!(transition.rtp_timestamp, Some(1));
    }

    #[test]
    fn continuation_submission_reuses_h264_bound_episode_when_latest_owner_slot_is_unavailable() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(make_render_frame(2), Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(1),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(180.0, 1, 55);
        sink.update(|stats| {
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 2001,
                    request_reason: Some("displaySupplyCritical".to_string()),
                    status: "recovering".to_string(),
                    requested_at_ms: 190.0,
                    ..Default::default()
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 41,
                    frame_rtp_timestamp: Some(2),
                    committed_sps_present: true,
                    committed_pps_present: true,
                    delta_continuation_ready: true,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
                    continuation_verdict: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    admission_accepted: true,
                    observed_at_ms: 195.0,
                    bound_episode_id: Some(1001),
                    bound_recovery_epoch: Some(1),
                    ..Default::default()
                });
        });

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(stats.latest_clean_anchor_submission_episode_id, Some(1001));
        assert_eq!(stats.latest_clean_anchor_submission_rtp_timestamp, Some(2));
    }

    #[test]
    fn continuation_submission_uses_recovery_owner_rtp_timestamp_when_frame_rtp_drifts() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        let mut frame = make_render_frame(2);
        frame.rtp_timestamp = Some(2);
        frame.recovery_owner_rtp_timestamp = Some(1);
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(frame, Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(1),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(180.0, 1, 55);
        sink.update(|stats| {
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 2001,
                    request_reason: Some("displaySupplyCritical".to_string()),
                    status: "recovering".to_string(),
                    requested_at_ms: 190.0,
                    ..Default::default()
                });
            stats.latest_h264_inspection_observation = None;
        });

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(stats.latest_clean_anchor_submission_episode_id, Some(1001));
        assert_eq!(stats.latest_clean_anchor_submission_rtp_timestamp, Some(2));
    }

    #[test]
    fn clean_anchor_submission_without_bound_episode_records_blocker_instead_of_episode_zero() {
        let mut state = XbxVideoDecodeState::new(20, 30).expect("decode state should initialize");
        let mut frame = make_render_frame(2);
        frame.rtp_timestamp = Some(2);
        state.enqueue_decoded_frame_with_clean_anchor_epoch_for_test(frame, Some(1));

        let runtime_stats = Arc::new(std::sync::Mutex::new(
            crate::XbxEngineMediaRuntimeStats::default(),
        ));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.begin_transport_recovery_episode(10.0);

        let drained = drain_pending_decoded_output_with_submit(&mut state, &sink, |_frame| {
            PendingDecodedSubmitResult::Submitted
        });

        assert!(drained.is_none());
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.latest_clean_anchor_submission_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_episode_id, None);
        let blocker = stats
            .latest_picture_recovery_blocker_observation
            .as_ref()
            .expect("clean anchor unbound blocker");
        assert_eq!(blocker.blocker_kind, "cleanAnchorEpisodeUnbound");
        assert_eq!(blocker.frame_rtp_timestamp, Some(2));
    }
}
