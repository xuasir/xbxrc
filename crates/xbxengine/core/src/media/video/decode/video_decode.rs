use std::collections::VecDeque;

use super::backend::{create_video_decoder_backend, XbxVideoDecoderBackend};
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
    ExternalResetRequested,
    BackendFailureEscalated,
    BootstrapKeyframeAccepted,
    RecoverySettled,
}

impl XbxVideoRecoveryEvent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExternalResetRequested => "external-reset-requested",
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

pub(crate) struct XbxVideoDecodeState {
    decoder: Box<dyn XbxVideoDecoderBackend>,
    latest_decoded_seq: u64,
    first_video_packet_logged: bool,
    decoded_frame_queue: VecDeque<DecodedFrame>,
    last_decode_ok_time_ms: Option<f64>,
    last_encoded_frame_time_ms: Option<f64>,
    decoder_reset_count: u64,
    latest_decoder_reset_time_ms: Option<f64>,
    decoded_frame_drop_count: u64,
    hardware_decode_failure_streak: u32,
    latest_hardware_decode_failure_time_ms: Option<f64>,
    latest_hardware_decode_failure_status: Option<i32>,
    recovery_state: XbxVideoRecoveryState,
    latest_recovery_state_change_time_ms: Option<f64>,
    latest_recovery_transition: Option<XbxVideoRecoveryTransitionSnapshot>,
    recovery_transition_id: u64,
    decode_candidate_state: XbxDecodeCandidateState,
    latest_decode_candidate_decision: Option<XbxDecodeCandidateDecisionSnapshot>,
    decode_candidate_decision_id: u64,
}

impl XbxVideoDecodeState {
    pub(crate) fn new(min_delay_ms: u64, max_delay_ms: u64) -> Result<Self, XbxEngineRuntimeError> {
        let _ = (min_delay_ms, max_delay_ms);
        Ok(Self {
            decoder: create_video_decoder_backend(),
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            recovery_state: XbxVideoRecoveryState::Nominal,
            latest_recovery_state_change_time_ms: None,
            latest_recovery_transition: None,
            recovery_transition_id: 0,
            decode_candidate_state: XbxDecodeCandidateState::Nominal,
            latest_decode_candidate_decision: None,
            decode_candidate_decision_id: 0,
        })
    }

    /**
     * 响应恢复控制面的 decoder reset：清空待释放队列，并重置硬解会话。
     * 这里不更改外部恢复阈值，只做局部状态收敛。
     */
    pub(crate) fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.decoded_frame_queue.clear();
        self.last_decode_ok_time_ms = None;
        self.decoder.reset()?;
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        let now_ms = now_ms_f64();
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.reset_hardware_failure_streak();
        self.transition_recovery_state(
            XbxVideoRecoveryState::WaitingKeyframe,
            XbxVideoRecoveryEvent::ExternalResetRequested,
            "decoderResetRequested",
            None,
            None,
            now_ms,
        );
        Ok(())
    }

    pub(crate) fn process_encoded_frame(
        &mut self,
        encoded_frame: EncodedFrame,
        now_ms: f64,
    ) -> Option<DecodedFrame> {
        self.last_encoded_frame_time_ms = Some(now_ms);
        if matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe)
            && !encoded_frame.h264.bootstrap_ready
        {
            return None;
        }
        if !self.first_video_packet_logged {
            self.first_video_packet_logged = true;
            crate::xbx_log_info!(
                "[xbxengine][rtc] first encoded video frame received ts={} bytes={}",
                encoded_frame.rtp_timestamp,
                encoded_frame.payload.len()
            );
        }
        let target_time = encoded_frame.target_playout_time;
        let rtp_timestamp = encoded_frame.rtp_timestamp;
        let is_keyframe = encoded_frame.is_keyframe;
        let budget = encoded_frame.budget;
        let frame_recovery_disposition = encoded_frame.frame_recovery_disposition;
        let frame_unrecoverable_reason = encoded_frame.frame_unrecoverable_reason.clone();
        let recovery_state_before_decode = self.recovery_state;
        let decoded_frame = match self.decoder.decode(encoded_frame, now_ms) {
            Ok(frame) => frame,
            Err(error) => {
                let status = parse_decoder_status_code(&error);
                crate::xbx_log_error!("[xbxengine][rtc] hardware decode failed: {error}");
                self.record_hardware_decode_failure(now_ms, status);
                if should_force_recovery_keyframe(status)
                    || self.hardware_decode_failure_streak >= 3
                {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc] decoder entered wait-keyframe recovery after backend failure"
                    );
                    let _ = self.request_decoder_reset_with_failure(status, now_ms);
                }
                return None;
            }
        };
        let Some(mut render_frame) = decoded_frame else {
            return None;
        };
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
        self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
        self.last_decode_ok_time_ms = Some(now_ms);
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
                crate::xbx_log_warn!(
                    "[xbxengine][vt] enqueue_decoded_frame: queue FULL, dropping old frame seq={}",
                    d.surface.frame_seq
                );
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

    fn request_decoder_reset_with_failure(
        &mut self,
        status: Option<i32>,
        now_ms: f64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.decoded_frame_queue.clear();
        self.last_decode_ok_time_ms = None;
        self.decoder.reset()?;
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.reset_hardware_failure_streak();
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
}

#[cfg(test)]
impl XbxVideoDecodeState {
    fn new_for_test(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxVideoDecoderBackend>,
    ) -> Self {
        let _ = (min_delay_ms, max_delay_ms);
        Self {
            decoder,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            recovery_state: XbxVideoRecoveryState::Nominal,
            latest_recovery_state_change_time_ms: None,
            latest_recovery_transition: None,
            recovery_transition_id: 0,
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
