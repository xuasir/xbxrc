use std::collections::VecDeque;

use super::backend::{
    create_video_decoder_backend_with_probe, XbxVideoDecoderBackend, XbxVideoDecoderProbeSummary,
};
#[cfg(test)]
use crate::media::video::render::renderer::XbxRenderFrame;
#[cfg(test)]
use crate::media::video::types::FrameRecoveryDisposition;
use crate::{
    media::video::types::{DecodedFrame, EncodedFrame},
    XbxEngineRuntimeError,
};

const MAX_DECODED_FRAME_QUEUE_LEN: usize = 2;
const HARDWARE_DECODE_FAILURE_BURST_GAP_MS: f64 = 400.0;
const LOCAL_DECODER_RESET_REPLAY_BARRIER_MS: f64 = 900.0;
const WAITING_KEYFRAME_CONTINUATION_WINDOW_MS: f64 = 120.0;
const WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES: u32 = 3;
type XbxVideoDecoderFactory =
    Box<dyn FnMut() -> (Box<dyn XbxVideoDecoderBackend>, XbxVideoDecoderProbeSummary) + Send>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxDecodeWorkloadState {
    AwaitingInput,
    DrainOutput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct XbxDecodeWorkloadSnapshot {
    pub(crate) state: XbxDecodeWorkloadState,
    pub(crate) pending_output_queue_depth: usize,
}

impl XbxDecodeWorkloadSnapshot {
    pub(crate) fn should_drain_output_first(self) -> bool {
        matches!(self.state, XbxDecodeWorkloadState::DrainOutput)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxDecodeCandidateState {
    Nominal,
    Backpressure,
}

impl XbxDecodeCandidateState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::Backpressure => "backpressure",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxDecodeIngressDemand {
    PullOutputFirst,
    AcceptInput,
}

impl XbxDecodeIngressDemand {
    pub(crate) fn should_pull_output_first(self) -> bool {
        matches!(self, Self::PullOutputFirst)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxVideoRecoveryState {
    Nominal,
    WaitingKeyframe,
    Recovering,
}

impl XbxVideoRecoveryState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::WaitingKeyframe => "waiting-keyframe",
            Self::Recovering => "recovering",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxVideoRecoveryEvent {
    ExternalDecoderResetRequested,
    BackendFailureEscalated,
    BootstrapKeyframeAccepted,
    RecoverySettled,
}

impl XbxVideoRecoveryEvent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExternalDecoderResetRequested => "external-decoder-reset-requested",
            Self::BackendFailureEscalated => "backend-failure-escalated",
            Self::BootstrapKeyframeAccepted => "bootstrap-keyframe-accepted",
            Self::RecoverySettled => "recovery-settled",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxDecodeCandidateDecisionSnapshot {
    pub(crate) decision_id: u64,
    pub(crate) state: XbxDecodeCandidateState,
    pub(crate) action: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) frame_seq: Option<u64>,
    pub(crate) observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxVideoRecoveryTransitionSnapshot {
    pub(crate) transition_id: u64,
    pub(crate) from_state: XbxVideoRecoveryState,
    pub(crate) to_state: XbxVideoRecoveryState,
    pub(crate) event: XbxVideoRecoveryEvent,
    pub(crate) detail: &'static str,
    pub(crate) frame_seq: Option<u64>,
    pub(crate) status: Option<i32>,
    pub(crate) observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxVideoDecoderProbeSnapshot {
    pub(crate) observation_id: u64,
    pub(crate) selected_backend_name: String,
    pub(crate) selected_backend_kind: String,
    pub(crate) fallback_count: u32,
    pub(crate) fallback_summary: Option<String>,
    pub(crate) observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxVideoDecoderBootstrapGateObservationSnapshot {
    pub(crate) observation_id: u64,
    pub(crate) recovery_state: XbxVideoRecoveryState,
    pub(crate) frame_rtp_timestamp: u32,
    pub(crate) is_idr: bool,
    pub(crate) has_inband_sps: bool,
    pub(crate) has_inband_pps: bool,
    pub(crate) committed_sps_present: bool,
    pub(crate) committed_pps_present: bool,
    pub(crate) bootstrap_ready: bool,
    pub(crate) bootstrap_reject_reason: Option<String>,
    pub(crate) observed_at_ms: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxDecodeOutputPathVerdict {
    BootstrapGateRejected,
    BackendError,
    BackendNoOutput,
    DecodedFrame,
}

impl XbxDecodeOutputPathVerdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BootstrapGateRejected => "bootstrap-gate-rejected",
            Self::BackendError => "backend-error",
            Self::BackendNoOutput => "backend-no-output",
            Self::DecodedFrame => "decoded-frame",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxDecodeOutputPathObservationSnapshot {
    pub(crate) observation_id: u64,
    pub(crate) verdict: XbxDecodeOutputPathVerdict,
    pub(crate) detail: &'static str,
    pub(crate) frame_rtp_timestamp: u32,
    pub(crate) is_keyframe: bool,
    pub(crate) status: Option<i32>,
    pub(crate) send_packet_status: Option<i32>,
    pub(crate) receive_frame_status: Option<i32>,
    pub(crate) backend_no_output_streak: Option<u32>,
    pub(crate) input_frames_since_last_decoded: Option<u32>,
    pub(crate) bootstrap_reject_reason: Option<String>,
    pub(crate) observed_at_ms: f64,
}

pub(crate) struct XbxVideoDecodeState {
    decoder: Box<dyn XbxVideoDecoderBackend>,
    decoder_factory: XbxVideoDecoderFactory,
    latest_decoded_seq: u64,
    first_video_packet_logged: bool,
    decoded_frame_queue: VecDeque<DecodedFrame>,
    last_decode_ok_time_ms: Option<f64>,
    last_encoded_frame_time_ms: Option<f64>,
    decoder_reset_count: u64,
    latest_decoder_reset_time_ms: Option<f64>,
    last_decoder_reset_success_edge_at_ms: Option<f64>,
    decoded_frame_drop_count: u64,
    hardware_decode_failure_streak: u32,
    latest_hardware_decode_failure_time_ms: Option<f64>,
    latest_hardware_decode_failure_status: Option<i32>,
    recovery_state: XbxVideoRecoveryState,
    latest_recovery_state_change_time_ms: Option<f64>,
    latest_recovery_transition: Option<XbxVideoRecoveryTransitionSnapshot>,
    recovery_transition_id: u64,
    latest_decoder_probe: Option<XbxVideoDecoderProbeSnapshot>,
    latest_bootstrap_gate_observation: Option<XbxVideoDecoderBootstrapGateObservationSnapshot>,
    bootstrap_gate_observation_id: u64,
    latest_decode_output_path_observation: Option<XbxDecodeOutputPathObservationSnapshot>,
    decode_output_path_observation_id: u64,
    backend_no_output_streak: u32,
    input_frames_since_last_decoded: u32,
    waiting_keyframe_continuation_deadline_ms: Option<f64>,
    waiting_keyframe_continuation_frames_left: u32,
    decode_candidate_state: XbxDecodeCandidateState,
    latest_decode_candidate_decision: Option<XbxDecodeCandidateDecisionSnapshot>,
    decode_candidate_decision_id: u64,
}

impl XbxVideoDecodeState {
    pub(crate) fn new(min_delay_ms: u64, max_delay_ms: u64) -> Result<Self, XbxEngineRuntimeError> {
        let _ = (min_delay_ms, max_delay_ms);
        let observed_at_ms = now_ms_f64();
        let mut decoder_factory: XbxVideoDecoderFactory =
            Box::new(create_video_decoder_backend_with_probe);
        let (decoder, probe) = decoder_factory();
        Ok(Self {
            decoder,
            decoder_factory,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            last_decoder_reset_success_edge_at_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            recovery_state: XbxVideoRecoveryState::Nominal,
            latest_recovery_state_change_time_ms: None,
            latest_recovery_transition: None,
            recovery_transition_id: 0,
            latest_decoder_probe: Some(Self::build_decoder_probe_snapshot(
                1,
                probe,
                observed_at_ms,
            )),
            latest_bootstrap_gate_observation: None,
            bootstrap_gate_observation_id: 0,
            latest_decode_output_path_observation: None,
            decode_output_path_observation_id: 0,
            backend_no_output_streak: 0,
            input_frames_since_last_decoded: 0,
            waiting_keyframe_continuation_deadline_ms: None,
            waiting_keyframe_continuation_frames_left: 0,
            decode_candidate_state: XbxDecodeCandidateState::Nominal,
            latest_decode_candidate_decision: None,
            decode_candidate_decision_id: 0,
        })
    }

    /**
     * 响应恢复控制面的 decoder reset：清空待释放队列，并重建本地解码 backend。
     * 这里不更改外部恢复阈值，只做局部状态收敛。
     */
    pub(crate) fn request_local_decoder_reset(&mut self) -> Result<bool, XbxEngineRuntimeError> {
        let now_ms = now_ms_f64();
        if self
            .latest_decoder_reset_time_ms
            .is_some_and(|last_reset_at_ms| {
                (now_ms - last_reset_at_ms).max(0.0) <= LOCAL_DECODER_RESET_REPLAY_BARRIER_MS
                    && self
                        .last_decoder_reset_success_edge_at_ms
                        .is_none_or(|success_at_ms| success_at_ms <= last_reset_at_ms)
            })
        {
            return Ok(false);
        }
        self.decoded_frame_queue.clear();
        self.last_decode_ok_time_ms = None;
        self.reset_decoder_backend(now_ms);
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.last_decoder_reset_success_edge_at_ms = None;
        self.reset_hardware_failure_streak();
        self.clear_waiting_keyframe_continuation();
        self.transition_recovery_state(
            XbxVideoRecoveryState::WaitingKeyframe,
            XbxVideoRecoveryEvent::ExternalDecoderResetRequested,
            "decoderResetRequested",
            None,
            None,
            now_ms,
        );
        Ok(true)
    }

    pub(crate) fn process_encoded_frame(
        &mut self,
        encoded_frame: EncodedFrame,
        now_ms: f64,
    ) -> Option<DecodedFrame> {
        self.last_encoded_frame_time_ms = Some(now_ms);
        let frame_rtp_timestamp = encoded_frame.rtp_timestamp;
        let frame_is_keyframe = encoded_frame.is_keyframe;
        let continuation_gate_bypassed =
            matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe)
                && !encoded_frame.h264.bootstrap_ready
                && self.try_consume_waiting_keyframe_continuation_allowance(&encoded_frame, now_ms);
        if matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe)
            && !encoded_frame.h264.bootstrap_ready
            && !continuation_gate_bypassed
        {
            let bootstrap_reject_reason = encoded_frame
                .h264
                .bootstrap_reject_reason
                .map(|reason| reason.as_str().to_string());
            self.record_bootstrap_gate_observation(&encoded_frame, now_ms);
            self.record_decode_output_path_observation(
                XbxDecodeOutputPathVerdict::BootstrapGateRejected,
                "bootstrapGateRejected",
                frame_rtp_timestamp,
                frame_is_keyframe,
                None,
                None,
                None,
                None,
                None,
                bootstrap_reject_reason,
                now_ms,
            );
            return None;
        }
        if !self.first_video_packet_logged {
            self.first_video_packet_logged = true;
        }
        self.input_frames_since_last_decoded =
            self.input_frames_since_last_decoded.saturating_add(1);
        let target_time = encoded_frame.target_playout_time;
        let rtp_timestamp = encoded_frame.rtp_timestamp;
        let is_keyframe = encoded_frame.is_keyframe;
        let budget = encoded_frame.budget;
        let frame_recovery_disposition = encoded_frame.frame_recovery_disposition;
        let frame_unrecoverable_reason = encoded_frame.frame_unrecoverable_reason.clone();
        let recovery_state_before_decode = self.recovery_state;
        let decode_outcome = match self.decoder.decode(encoded_frame, now_ms) {
            Ok(outcome) => outcome,
            Err(error) => {
                if matches!(
                    recovery_state_before_decode,
                    XbxVideoRecoveryState::WaitingKeyframe
                ) && frame_is_keyframe
                {
                    self.clear_waiting_keyframe_continuation();
                }
                let status = parse_decoder_status_code(&error);
                self.record_hardware_decode_failure(now_ms, status);
                self.backend_no_output_streak = 0;
                self.record_decode_output_path_observation(
                    XbxDecodeOutputPathVerdict::BackendError,
                    "backendError",
                    frame_rtp_timestamp,
                    frame_is_keyframe,
                    status,
                    None,
                    None,
                    None,
                    Some(self.input_frames_since_last_decoded),
                    None,
                    now_ms,
                );
                if self.hardware_decode_failure_streak == 1 {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc] hardware decode failed status={:?} err={error}",
                        status
                    );
                }
                if should_force_recovery_keyframe(status)
                    || self.hardware_decode_failure_streak >= 3
                {
                    let _ = self.reset_decoder_with_failure(status, now_ms);
                }
                return None;
            }
        };
        let decoded_frame = decode_outcome.frame;
        let Some(mut render_frame) = decoded_frame else {
            let waiting_keyframe_bootstrap_no_output = matches!(
                recovery_state_before_decode,
                XbxVideoRecoveryState::WaitingKeyframe
            ) && frame_is_keyframe;
            if waiting_keyframe_bootstrap_no_output {
                self.arm_waiting_keyframe_continuation(now_ms);
            }
            self.backend_no_output_streak = self.backend_no_output_streak.saturating_add(1);
            self.record_decode_output_path_observation(
                XbxDecodeOutputPathVerdict::BackendNoOutput,
                if continuation_gate_bypassed {
                    "backendNoOutputAfterContinuationBypass"
                } else if waiting_keyframe_bootstrap_no_output {
                    "backendNoOutputAfterBootstrapKeyframe"
                } else {
                    "backendNoOutput"
                },
                frame_rtp_timestamp,
                frame_is_keyframe,
                None,
                decode_outcome.send_packet_status,
                decode_outcome.receive_frame_status,
                Some(self.backend_no_output_streak),
                Some(self.input_frames_since_last_decoded),
                None,
                now_ms,
            );
            return None;
        };
        if matches!(
            recovery_state_before_decode,
            XbxVideoRecoveryState::WaitingKeyframe
        ) && frame_is_keyframe
        {
            self.clear_waiting_keyframe_continuation();
        }
        if self
            .latest_decoder_reset_time_ms
            .is_some_and(|last_reset_at_ms| now_ms > last_reset_at_ms)
        {
            self.last_decoder_reset_success_edge_at_ms = Some(now_ms);
        }
        match recovery_state_before_decode {
            XbxVideoRecoveryState::WaitingKeyframe => self.transition_recovery_state(
                XbxVideoRecoveryState::Recovering,
                XbxVideoRecoveryEvent::BootstrapKeyframeAccepted,
                "bootstrapKeyframeDecoded",
                Some(rtp_timestamp as u64),
                None,
                now_ms,
            ),
            XbxVideoRecoveryState::Recovering => self.transition_recovery_state(
                XbxVideoRecoveryState::Nominal,
                XbxVideoRecoveryEvent::RecoverySettled,
                "recoverySettled",
                Some(rtp_timestamp as u64),
                None,
                now_ms,
            ),
            XbxVideoRecoveryState::Nominal => {}
        }
        self.reset_hardware_failure_streak();
        self.backend_no_output_streak = 0;
        self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
        self.last_decode_ok_time_ms = Some(now_ms);
        self.record_decode_output_path_observation(
            XbxDecodeOutputPathVerdict::DecodedFrame,
            if continuation_gate_bypassed {
                "decodedFrameReadyAfterContinuationBypass"
            } else {
                "decodedFrameReady"
            },
            frame_rtp_timestamp,
            frame_is_keyframe,
            None,
            decode_outcome.send_packet_status,
            decode_outcome.receive_frame_status,
            Some(self.backend_no_output_streak),
            Some(self.input_frames_since_last_decoded),
            None,
            now_ms,
        );
        self.input_frames_since_last_decoded = 0;
        render_frame.frame_seq = self.latest_decoded_seq;
        render_frame.rendered_at_ms = now_ms;
        render_frame.rtp_timestamp = Some(rtp_timestamp);
        render_frame.is_keyframe = is_keyframe;
        render_frame.frame_recovery_disposition =
            Some(frame_recovery_disposition.as_str().to_string());
        render_frame.frame_unrecoverable_reason = frame_unrecoverable_reason.clone();
        self.enqueue_decoded_frame(DecodedFrame {
            pts: target_time,
            rtp_timestamp,
            is_keyframe,
            budget,
            frame_recovery_disposition,
            frame_unrecoverable_reason,
            surface: render_frame,
        })
    }

    pub(crate) fn last_decode_ok_time_ms(&self) -> Option<f64> {
        self.last_decode_ok_time_ms
    }

    pub(crate) fn decoder_backend_name(&self) -> &'static str {
        self.decoder.backend_name()
    }

    pub(crate) fn decoder_reset_count(&self) -> u64 {
        self.decoder_reset_count
    }

    pub(crate) fn latest_decoder_reset_time_ms(&self) -> Option<f64> {
        self.latest_decoder_reset_time_ms
    }

    pub(crate) fn decoded_frame_drop_count(&self) -> u64 {
        self.decoded_frame_drop_count
    }

    pub(crate) fn hardware_decode_failure_streak(&self) -> u32 {
        self.hardware_decode_failure_streak
    }

    pub(crate) fn latest_hardware_decode_failure_time_ms(&self) -> Option<f64> {
        self.latest_hardware_decode_failure_time_ms
    }

    pub(crate) fn latest_hardware_decode_failure_status(&self) -> Option<i32> {
        self.latest_hardware_decode_failure_status
    }

    pub(crate) fn recovery_state(&self) -> XbxVideoRecoveryState {
        self.recovery_state
    }

    pub(crate) fn latest_recovery_state_change_time_ms(&self) -> Option<f64> {
        self.latest_recovery_state_change_time_ms
    }

    pub(crate) fn latest_recovery_transition(&self) -> Option<&XbxVideoRecoveryTransitionSnapshot> {
        self.latest_recovery_transition.as_ref()
    }

    pub(crate) fn latest_decoder_probe(&self) -> Option<&XbxVideoDecoderProbeSnapshot> {
        self.latest_decoder_probe.as_ref()
    }

    pub(crate) fn latest_bootstrap_gate_observation(
        &self,
    ) -> Option<&XbxVideoDecoderBootstrapGateObservationSnapshot> {
        self.latest_bootstrap_gate_observation.as_ref()
    }

    pub(crate) fn latest_decode_output_path_observation(
        &self,
    ) -> Option<&XbxDecodeOutputPathObservationSnapshot> {
        self.latest_decode_output_path_observation.as_ref()
    }

    pub(crate) fn latest_decode_candidate_decision(
        &self,
    ) -> Option<&XbxDecodeCandidateDecisionSnapshot> {
        self.latest_decode_candidate_decision.as_ref()
    }

    pub(crate) fn pop_decoded_frame(&mut self, _now_ms: f64) -> Option<DecodedFrame> {
        // native 路径已经有 pacer 负责 playout 节奏，decode stage 不再额外等待。
        self.decoded_frame_queue.pop_front()
    }

    pub(crate) fn has_decoded_frame(&self) -> bool {
        !self.decoded_frame_queue.is_empty()
    }

    pub(crate) fn workload_snapshot(&self) -> XbxDecodeWorkloadSnapshot {
        let pending_output_queue_depth = self.decoded_frame_queue.len();
        let state = if pending_output_queue_depth > 0 {
            XbxDecodeWorkloadState::DrainOutput
        } else {
            XbxDecodeWorkloadState::AwaitingInput
        };
        XbxDecodeWorkloadSnapshot {
            state,
            pending_output_queue_depth,
        }
    }

    pub(crate) fn ingress_demand(&self) -> XbxDecodeIngressDemand {
        if self.workload_snapshot().should_drain_output_first() {
            XbxDecodeIngressDemand::PullOutputFirst
        } else {
            XbxDecodeIngressDemand::AcceptInput
        }
    }

    #[cfg(test)]
    pub(crate) fn peek_decoded_frame(&self) -> Option<&DecodedFrame> {
        self.decoded_frame_queue.front()
    }

    pub(crate) fn decoded_frame_queue_len(&self) -> usize {
        self.decoded_frame_queue.len()
    }

    #[cfg(test)]
    pub(crate) fn decoded_frame_queue_is_full(&self) -> bool {
        self.decoded_frame_queue.len() >= MAX_DECODED_FRAME_QUEUE_LEN
    }

    pub(crate) fn requeue_decoded_frame_front(&mut self, frame: DecodedFrame) {
        self.decoded_frame_queue.push_front(frame);
    }

    fn enqueue_decoded_frame(&mut self, frame: DecodedFrame) -> Option<DecodedFrame> {
        let incoming_frame_seq = frame.surface.frame_seq;
        let observed_at_ms = frame.surface.rendered_at_ms;
        let mut dropped_frame = None;
        while self.decoded_frame_queue.len() >= MAX_DECODED_FRAME_QUEUE_LEN {
            let dropped = self.decoded_frame_queue.pop_front();
            if let Some(d) = dropped {
                dropped_frame = Some(d);
            }
            self.decoded_frame_drop_count = self.decoded_frame_drop_count.saturating_add(1);
        }
        self.decoded_frame_queue.push_back(frame);
        if let Some(dropped) = dropped_frame.as_ref() {
            self.record_decode_candidate_decision(
                XbxDecodeCandidateState::Backpressure,
                "drop",
                "outputQueueOverflow",
                Some(dropped.surface.frame_seq),
                observed_at_ms,
            );
        } else if matches!(
            self.decode_candidate_state,
            XbxDecodeCandidateState::Backpressure
        ) {
            self.record_decode_candidate_decision(
                XbxDecodeCandidateState::Nominal,
                "accept",
                "queueRecovered",
                Some(incoming_frame_seq),
                observed_at_ms,
            );
        }
        dropped_frame
    }

    // 连续硬解失败用于 recovery 诊断：只在短窗口内累加，避免偶发错误误触发。
    fn record_hardware_decode_failure(&mut self, now_ms: f64, status: Option<i32>) {
        let same_burst = self
            .latest_hardware_decode_failure_time_ms
            .map(|last| (now_ms - last).max(0.0) <= HARDWARE_DECODE_FAILURE_BURST_GAP_MS)
            .unwrap_or(false);
        self.hardware_decode_failure_streak = if same_burst {
            self.hardware_decode_failure_streak.saturating_add(1)
        } else {
            1
        };
        self.latest_hardware_decode_failure_time_ms = Some(now_ms);
        self.latest_hardware_decode_failure_status = status;
    }

    fn reset_hardware_failure_streak(&mut self) {
        self.hardware_decode_failure_streak = 0;
        self.latest_hardware_decode_failure_status = None;
    }

    fn reset_decoder_with_failure(
        &mut self,
        status: Option<i32>,
        now_ms: f64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.decoded_frame_queue.clear();
        self.last_decode_ok_time_ms = None;
        self.reset_decoder_backend(now_ms);
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.reset_hardware_failure_streak();
        self.backend_no_output_streak = 0;
        self.input_frames_since_last_decoded = 0;
        self.clear_waiting_keyframe_continuation();
        self.transition_recovery_state(
            XbxVideoRecoveryState::WaitingKeyframe,
            XbxVideoRecoveryEvent::BackendFailureEscalated,
            "backendFailureReset",
            None,
            status,
            now_ms,
        );
        Ok(())
    }

    fn reset_decoder_backend(&mut self, observed_at_ms: f64) {
        let (decoder, probe) = (self.decoder_factory)();
        self.decoder = decoder;
        let next_observation_id = self
            .latest_decoder_probe
            .as_ref()
            .map(|probe| probe.observation_id.saturating_add(1))
            .unwrap_or(1);
        self.latest_decoder_probe = Some(Self::build_decoder_probe_snapshot(
            next_observation_id,
            probe,
            observed_at_ms,
        ));
    }

    fn transition_recovery_state(
        &mut self,
        to_state: XbxVideoRecoveryState,
        event: XbxVideoRecoveryEvent,
        detail: &'static str,
        frame_seq: Option<u64>,
        status: Option<i32>,
        observed_at_ms: f64,
    ) {
        let from_state = self.recovery_state;
        self.recovery_state = to_state;
        self.recovery_transition_id = self.recovery_transition_id.saturating_add(1);
        self.latest_recovery_state_change_time_ms = Some(observed_at_ms);
        self.latest_recovery_transition = Some(XbxVideoRecoveryTransitionSnapshot {
            transition_id: self.recovery_transition_id,
            from_state,
            to_state,
            event,
            detail,
            frame_seq,
            status,
            observed_at_ms,
        });
    }

    fn record_decode_candidate_decision(
        &mut self,
        state: XbxDecodeCandidateState,
        action: &'static str,
        detail: &'static str,
        frame_seq: Option<u64>,
        observed_at_ms: f64,
    ) {
        self.decode_candidate_state = state;
        self.decode_candidate_decision_id = self.decode_candidate_decision_id.saturating_add(1);
        self.latest_decode_candidate_decision = Some(XbxDecodeCandidateDecisionSnapshot {
            decision_id: self.decode_candidate_decision_id,
            state,
            action,
            detail,
            frame_seq,
            observed_at_ms,
        });
    }

    fn build_decoder_probe_snapshot(
        observation_id: u64,
        probe: XbxVideoDecoderProbeSummary,
        observed_at_ms: f64,
    ) -> XbxVideoDecoderProbeSnapshot {
        XbxVideoDecoderProbeSnapshot {
            observation_id,
            selected_backend_name: probe.selected_backend_name,
            selected_backend_kind: probe.selected_backend_kind,
            fallback_count: probe.fallback_count,
            fallback_summary: probe.fallback_summary,
            observed_at_ms,
        }
    }

    fn record_bootstrap_gate_observation(
        &mut self,
        encoded_frame: &EncodedFrame,
        observed_at_ms: f64,
    ) {
        self.bootstrap_gate_observation_id = self.bootstrap_gate_observation_id.saturating_add(1);
        self.latest_bootstrap_gate_observation =
            Some(XbxVideoDecoderBootstrapGateObservationSnapshot {
                observation_id: self.bootstrap_gate_observation_id,
                recovery_state: self.recovery_state,
                frame_rtp_timestamp: encoded_frame.rtp_timestamp,
                is_idr: encoded_frame.h264.is_idr,
                has_inband_sps: encoded_frame.h264.has_inband_sps,
                has_inband_pps: encoded_frame.h264.has_inband_pps,
                committed_sps_present: encoded_frame.h264.committed_sps_present(),
                committed_pps_present: encoded_frame.h264.committed_pps_present(),
                bootstrap_ready: encoded_frame.h264.bootstrap_ready,
                bootstrap_reject_reason: encoded_frame
                    .h264
                    .bootstrap_reject_reason
                    .map(|reason| reason.as_str().to_string()),
                observed_at_ms,
            });
    }

    fn record_decode_output_path_observation(
        &mut self,
        verdict: XbxDecodeOutputPathVerdict,
        detail: &'static str,
        frame_rtp_timestamp: u32,
        is_keyframe: bool,
        status: Option<i32>,
        send_packet_status: Option<i32>,
        receive_frame_status: Option<i32>,
        backend_no_output_streak: Option<u32>,
        input_frames_since_last_decoded: Option<u32>,
        bootstrap_reject_reason: Option<String>,
        observed_at_ms: f64,
    ) {
        self.decode_output_path_observation_id =
            self.decode_output_path_observation_id.saturating_add(1);
        self.latest_decode_output_path_observation = Some(XbxDecodeOutputPathObservationSnapshot {
            observation_id: self.decode_output_path_observation_id,
            verdict,
            detail,
            frame_rtp_timestamp,
            is_keyframe,
            status,
            send_packet_status,
            receive_frame_status,
            backend_no_output_streak,
            input_frames_since_last_decoded,
            bootstrap_reject_reason,
            observed_at_ms,
        });
    }

    fn arm_waiting_keyframe_continuation(&mut self, now_ms: f64) {
        self.waiting_keyframe_continuation_deadline_ms =
            Some(now_ms + WAITING_KEYFRAME_CONTINUATION_WINDOW_MS);
        self.waiting_keyframe_continuation_frames_left = WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES;
    }

    fn clear_waiting_keyframe_continuation(&mut self) {
        self.waiting_keyframe_continuation_deadline_ms = None;
        self.waiting_keyframe_continuation_frames_left = 0;
    }

    fn try_consume_waiting_keyframe_continuation_allowance(
        &mut self,
        encoded_frame: &EncodedFrame,
        now_ms: f64,
    ) -> bool {
        let within_window = self
            .waiting_keyframe_continuation_deadline_ms
            .is_some_and(|deadline_ms| now_ms <= deadline_ms);
        if !within_window || self.waiting_keyframe_continuation_frames_left == 0 {
            self.clear_waiting_keyframe_continuation();
            return false;
        }
        // 仅放行可安全承接 bootstrap keyframe 的 continuation。
        if !encoded_frame.h264.delta_continuation_ready()
            || !encoded_frame.h264.committed_sps_present()
            || !encoded_frame.h264.committed_pps_present()
        {
            return false;
        }
        self.waiting_keyframe_continuation_frames_left = self
            .waiting_keyframe_continuation_frames_left
            .saturating_sub(1);
        true
    }
}

#[cfg(test)]
impl XbxVideoDecodeState {
    fn new_for_test(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxVideoDecoderBackend>,
    ) -> Self {
        Self::new_for_test_with_factory(
            min_delay_ms,
            max_delay_ms,
            decoder,
            Box::new(|| {
                panic!("test decoder factory was not configured for decoder reset path");
            }),
        )
    }

    fn new_for_test_with_factory(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxVideoDecoderBackend>,
        decoder_factory: XbxVideoDecoderFactory,
    ) -> Self {
        let _ = (min_delay_ms, max_delay_ms);
        Self {
            decoder,
            decoder_factory,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            last_decoder_reset_success_edge_at_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            recovery_state: XbxVideoRecoveryState::Nominal,
            latest_recovery_state_change_time_ms: None,
            latest_recovery_transition: None,
            recovery_transition_id: 0,
            latest_decoder_probe: None,
            latest_bootstrap_gate_observation: None,
            bootstrap_gate_observation_id: 0,
            latest_decode_output_path_observation: None,
            decode_output_path_observation_id: 0,
            backend_no_output_streak: 0,
            input_frames_since_last_decoded: 0,
            waiting_keyframe_continuation_deadline_ms: None,
            waiting_keyframe_continuation_frames_left: 0,
            decode_candidate_state: XbxDecodeCandidateState::Nominal,
            latest_decode_candidate_decision: None,
            decode_candidate_decision_id: 0,
        }
    }

    pub(crate) fn enqueue_decoded_frame_for_test(&mut self, frame: XbxRenderFrame) {
        let _ = self.enqueue_decoded_frame(DecodedFrame {
            pts: std::time::Instant::now(),
            rtp_timestamp: frame.frame_seq as u32,
            is_keyframe: frame.is_keyframe,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: frame,
        });
    }
}

pub(crate) fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn parse_decoder_status_code(error: &XbxEngineRuntimeError) -> Option<i32> {
    let message = error.to_string();
    let status = message.split("status=").nth(1)?;
    let token = status
        .split(|ch: char| !(ch == '-' || ch.is_ascii_digit()))
        .next()?;
    token.parse::<i32>().ok()
}

fn should_force_recovery_keyframe(status: Option<i32>) -> bool {
    matches!(
        status,
        Some(K_VT_VIDEO_DECODER_BAD_DATA_ERR | K_VT_VIDEO_DECODER_REFERENCE_MISSING_ERR)
    )
}
const K_VT_VIDEO_DECODER_BAD_DATA_ERR: i32 = -12909;
const K_VT_VIDEO_DECODER_REFERENCE_MISSING_ERR: i32 = -17694;

#[cfg(test)]
#[path = "video_decode.test.rs"]
mod tests;
