use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use super::backend::{
    create_software_video_decoder_backend_with_probe, create_video_decoder_backend_with_probe,
    XbxVideoDecoderBackend, XbxVideoDecoderProbeSummary,
};
use crate::media::video::h264::inspection::H264BootstrapRejectReason;
use crate::media::video::ingress::budget::FrameBudgetWindowSource;
#[cfg(test)]
use crate::media::video::render::renderer::XbxRenderFrame;
#[cfg(test)]
use crate::media::video::types::FrameRecoveryDisposition;
use crate::{
    media::video::types::{
        decoded_presentation_value_role, derive_presentation_value_role, DecodedFrame, EncodedFrame,
    },
    XbxEngineRuntimeError,
};

const DECODE_OUTPUT_MAILBOX_CAPACITY: usize = 2;
const HARDWARE_DECODE_FAILURE_BURST_GAP_MS: f64 = 400.0;
const HARDWARE_NO_OUTPUT_SOFT_FALLBACK_THRESHOLD: u32 = 2;
const D3D11VA_NO_OUTPUT_REBUILD_THRESHOLD: u32 = 2;
const D3D11VA_NO_OUTPUT_MAX_REBUILD_ATTEMPTS: u32 = 1;
const NOMINAL_CONTINUATION_NO_OUTPUT_RECOVERY_THRESHOLD: u32 = 2;
const RECOVERING_CONTINUATION_NO_OUTPUT_RECOVERY_THRESHOLD: u32 = 2;
const CONTINUATION_NO_OUTPUT_KEYFRAME_HINT_THRESHOLD: u32 = 1;
// 首帧阶段硬解不出帧：不要死等（以毫秒窗作为上限）。
const HARDWARE_NO_OUTPUT_SOFT_FALLBACK_WINDOW_MS: f64 = 80.0;
const LOCAL_DECODER_RESET_REPLAY_BARRIER_MS: f64 = 900.0;
const WAITING_KEYFRAME_CONTINUATION_WINDOW_MS: f64 = 120.0;
const WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES: u32 = 3;
const TIMED_FALLBACK_DISPLAYED_IDR_CONTINUATION_WINDOW_MS: f64 = 4_000.0;
const TIMED_FALLBACK_DISPLAYED_IDR_CONTINUATION_MAX_FRAMES: u32 = 120;
const DECODE_QUEUE_STALE_SLACK_DISPOSABLE_MS: u64 = 24;
const DECODE_QUEUE_STALE_SLACK_SUPPLY_MS: u64 = 48;
const DECODE_QUEUE_STALE_SLACK_ANCHOR_MS: u64 = 72;
const DECODE_QUEUE_STALE_SLACK_RECOVERY_BONUS_MS: u64 = 48;
const DECODE_QUEUE_STALE_SLACK_GUARD_MS: u64 = 2;
const DECODE_MAILBOX_REPLACE_MIN_INTERVAL_FLOOR_MS: f64 = 12.0;
const DECODE_MAILBOX_REPLACE_MIN_INTERVAL_CEILING_MS: f64 = 100.0;
/// gap/recovering 上连续 INVALIDDATA 后抑制 nominal continuation decoder reset。
const TRANSIENT_DECODE_ERROR_RESET_COALESCE_MS: f64 = 800.0;
type XbxVideoDecoderFactory =
    Box<dyn FnMut() -> (Box<dyn XbxVideoDecoderBackend>, XbxVideoDecoderProbeSummary) + Send>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum XbxDecodeWorkloadState {
    AwaitingInput,
    DrainOutput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct XbxDecodeWorkloadSnapshot {
    pub(crate) state: XbxDecodeWorkloadState,
    pub(crate) pending_output_queue_depth: usize,
}

impl XbxDecodeWorkloadSnapshot {
    #[allow(dead_code)]
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

fn recovery_chain_unsettled(
    recovery_epoch_tag: Option<u64>,
    frame_recovery_disposition: crate::media::video::types::FrameRecoveryDisposition,
) -> bool {
    recovery_epoch_tag.is_some()
        && !matches!(
            frame_recovery_disposition,
            crate::media::video::types::FrameRecoveryDisposition::Steady
        )
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
    pub(crate) replacement_decision:
        Option<crate::api::backend::XbxEngineReplacementDecisionObservation>,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxRemoteFrameCaptureObservationSnapshot {
    pub(crate) observation_id: u64,
    pub(crate) trigger: &'static str,
    pub(crate) backend_name: String,
    pub(crate) frame_rtp_timestamp: u32,
    pub(crate) is_keyframe: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) payload_bytes: usize,
    pub(crate) payload_fingerprint: u64,
    pub(crate) payload_prefix_hex: String,
    pub(crate) nal_types: Vec<String>,
    pub(crate) nal_count: u16,
    pub(crate) has_inband_sps: bool,
    pub(crate) has_inband_pps: bool,
    pub(crate) bootstrap_ready: bool,
    pub(crate) bootstrap_reject_reason: Option<String>,
    pub(crate) parameter_sets_changed: bool,
    pub(crate) config_changed: bool,
    pub(crate) slice_headers_valid: bool,
    pub(crate) send_packet_status: Option<i32>,
    pub(crate) receive_frame_status: Option<i32>,
    pub(crate) status: Option<i32>,
    pub(crate) backend_no_output_streak: Option<u32>,
    pub(crate) input_frames_since_last_decoded: Option<u32>,
    pub(crate) observed_at_ms: f64,
}

pub(crate) struct XbxVideoDecodeState {
    decoder: Box<dyn XbxVideoDecoderBackend>,
    decoder_factory: XbxVideoDecoderFactory,
    software_decoder_factory: XbxVideoDecoderFactory,
    latest_decoded_seq: u64,
    first_video_packet_logged: bool,
    decoded_inflight_current: Option<DecodedFrame>,
    decoded_latest_candidate: Option<DecodedFrame>,
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
    latest_remote_frame_capture_observation: Option<XbxRemoteFrameCaptureObservationSnapshot>,
    remote_frame_capture_observation_id: u64,
    backend_no_output_streak: u32,
    input_frames_since_last_decoded: u32,
    first_hardware_no_output_at_ms: Option<f64>,
    waiting_keyframe_continuation_deadline_ms: Option<f64>,
    waiting_keyframe_continuation_frames_left: u32,
    decode_candidate_state: XbxDecodeCandidateState,
    latest_decode_candidate_decision: Option<XbxDecodeCandidateDecisionSnapshot>,
    decode_candidate_decision_id: u64,
    d3d11va_no_output_rebuild_attempts: u32,
    nominal_continuation_hw_no_output_resets: u32,
    last_decode_latest_replace_at_ms: Option<f64>,
    /// 与 pacer 共用的呈现节拍尺（host 刷新与流帧率取较慢侧）。
    mailbox_present_cadence_interval_ms: f64,
    /// TimedFallback + displayed-idr：允许 bootstrapMissingIdr 的 delta 进入解码器续播。
    timed_fallback_displayed_idr_bypass: bool,
    /// 与 InsertGate Emit 对齐：ingress 已裁决可提交的 soft continuation。
    insert_emit_bootstrap_bypass: bool,
    /// nominal/recovering continuation 无输出 reset 后由 actor 写入 receive PLI hint。
    pending_receive_keyframe_hint_at_ms: Option<f64>,
    /// 近期 delta 上 AVERROR_INVALIDDATA/EAGAIN：抑制 gap 内误触发 decoder reset。
    last_transient_decode_error_at_ms: Option<f64>,
}

impl XbxVideoDecodeState {
    pub(crate) fn new(min_delay_ms: u64, max_delay_ms: u64) -> Result<Self, XbxEngineRuntimeError> {
        let _ = (min_delay_ms, max_delay_ms);
        let observed_at_ms = now_ms_f64();
        let mut decoder_factory: XbxVideoDecoderFactory =
            Box::new(create_video_decoder_backend_with_probe);
        let software_decoder_factory: XbxVideoDecoderFactory =
            Box::new(create_software_video_decoder_backend_with_probe);
        let (decoder, probe) = decoder_factory();
        Ok(Self {
            decoder,
            decoder_factory,
            software_decoder_factory,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_inflight_current: None,
            decoded_latest_candidate: None,
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
            latest_remote_frame_capture_observation: None,
            remote_frame_capture_observation_id: 0,
            backend_no_output_streak: 0,
            input_frames_since_last_decoded: 0,
            first_hardware_no_output_at_ms: None,
            waiting_keyframe_continuation_deadline_ms: None,
            waiting_keyframe_continuation_frames_left: 0,
            decode_candidate_state: XbxDecodeCandidateState::Nominal,
            latest_decode_candidate_decision: None,
            decode_candidate_decision_id: 0,
            d3d11va_no_output_rebuild_attempts: 0,
            nominal_continuation_hw_no_output_resets: 0,
            last_decode_latest_replace_at_ms: None,
            mailbox_present_cadence_interval_ms:
                crate::media::video::present_cadence::PRESENT_CADENCE_INTERVAL_FALLBACK_MS,
            timed_fallback_displayed_idr_bypass: false,
            insert_emit_bootstrap_bypass: false,
            pending_receive_keyframe_hint_at_ms: None,
            last_transient_decode_error_at_ms: None,
        })
    }

    pub(crate) fn set_insert_emit_bootstrap_bypass(&mut self, bypass: bool) {
        self.insert_emit_bootstrap_bypass = bypass;
    }

    pub(crate) fn take_pending_receive_keyframe_hint_at_ms(&mut self) -> Option<f64> {
        self.pending_receive_keyframe_hint_at_ms.take()
    }

    /// 与 recovery 合同同步：TimedFallback 时打通 displayed-idr delta 解码续播。
    pub(crate) fn sync_recovery_exit_policy_from_stats(
        &mut self,
        stats: &crate::XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) {
        use crate::transport::rtc::recovery::contract::{
            displayed_idr_serving_from_stats, has_current_clean_anchor_from_stats,
            recovery_supply_break_active_from_stats, recovery_timed_fallback_active_from_stats,
        };
        let bypass = displayed_idr_serving_from_stats(stats)
            && (recovery_timed_fallback_active_from_stats(stats, now_ms)
                || (has_current_clean_anchor_from_stats(stats)
                    && recovery_supply_break_active_from_stats(stats, now_ms)));
        if bypass && !self.timed_fallback_displayed_idr_bypass {
            self.waiting_keyframe_continuation_deadline_ms =
                Some(now_ms + TIMED_FALLBACK_DISPLAYED_IDR_CONTINUATION_WINDOW_MS);
            self.waiting_keyframe_continuation_frames_left =
                TIMED_FALLBACK_DISPLAYED_IDR_CONTINUATION_MAX_FRAMES;
        }
        self.timed_fallback_displayed_idr_bypass = bypass;
    }

    pub(crate) fn set_mailbox_present_cadence(&mut self, present_cadence_interval_ms: f64) {
        self.mailbox_present_cadence_interval_ms = present_cadence_interval_ms.clamp(
            DECODE_MAILBOX_REPLACE_MIN_INTERVAL_FLOOR_MS,
            DECODE_MAILBOX_REPLACE_MIN_INTERVAL_CEILING_MS,
        );
    }

    fn resolve_decode_mailbox_replace_min_interval_ms(&self) -> f64 {
        self.mailbox_present_cadence_interval_ms
    }

    /// 仅在保护已提交的 anchor 候选、抵御恢复窗内 continuation 突发时使用；steady supply 走 value supersede。
    fn should_protect_anchor_candidate_from_incoming_burst(
        &self,
        incoming: &DecodedFrame,
        existing: &DecodedFrame,
        observed_at_ms: f64,
    ) -> bool {
        let Some(last_replace_at_ms) = self.last_decode_latest_replace_at_ms else {
            return false;
        };
        if observed_at_ms - last_replace_at_ms
            >= self.resolve_decode_mailbox_replace_min_interval_ms()
        {
            return false;
        }
        if incoming.is_keyframe {
            return false;
        }
        if existing.is_keyframe && !incoming.is_keyframe {
            let incoming_can_upgrade_anchor =
                matches!(
                    decoded_presentation_value_role(incoming),
                    crate::api::backend::XbxEnginePresentationValueRole::FreshAnchor
                        | crate::api::backend::XbxEnginePresentationValueRole::RecoveryContinuation
                ) && Self::compare_decoded_candidate_value(incoming, existing) > 0;
            if incoming_can_upgrade_anchor {
                return false;
            }
            return true;
        }
        if Self::compare_decoded_candidate_value(incoming, existing) > 0 {
            return false;
        }
        matches!(existing.budget.recovery_value_tier(), "anchor")
            || matches!(
                decoded_presentation_value_role(existing),
                crate::api::backend::XbxEnginePresentationValueRole::FreshAnchor
                    | crate::api::backend::XbxEnginePresentationValueRole::RecoveryContinuation
            )
    }

    /**
     * 响应恢复控制面的 decoder reset：清空待释放队列，并重建本地解码 backend。
     * 这里不更改外部恢复阈值，只做局部状态收敛。
     */
    pub(crate) fn request_local_decoder_reset(&mut self) -> Result<bool, XbxEngineRuntimeError> {
        let now_ms = now_ms_f64();
        if self.should_suppress_repeat_waiting_keyframe_decoder_reset(now_ms) {
            return Ok(false);
        }
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
        self.clear_decoded_output_mailbox();
        self.last_decode_ok_time_ms = None;
        self.reset_decoder_backend(now_ms);
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.last_decoder_reset_success_edge_at_ms = None;
        self.reset_hardware_failure_streak();
        self.clear_waiting_keyframe_continuation();
        self.first_hardware_no_output_at_ms = None;
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
        if !self.decoder_backend_is_d3d11va() {
            self.d3d11va_no_output_rebuild_attempts = 0;
        }
        self.last_encoded_frame_time_ms = Some(now_ms);
        let frame_rtp_timestamp = encoded_frame.rtp_timestamp;
        let frame_is_keyframe = encoded_frame.is_keyframe;
        let timed_fallback_bypass = self.timed_fallback_displayed_idr_bypass
            && !encoded_frame.h264.bootstrap_ready
            && encoded_frame.h264.delta_continuation_ready()
            && encoded_frame.h264.committed_sps_present()
            && encoded_frame.h264.committed_pps_present();
        let waiting_keyframe_continuation_allowed =
            matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe)
                && !encoded_frame.h264.bootstrap_ready
                && (timed_fallback_bypass
                    || self.try_consume_waiting_keyframe_continuation_allowance(
                        &encoded_frame,
                        now_ms,
                    ));
        let continuation_gate_bypassed = timed_fallback_bypass || self.insert_emit_bootstrap_bypass;
        if matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe)
            && !encoded_frame.h264.bootstrap_ready
            && !waiting_keyframe_continuation_allowed
            && !continuation_gate_bypassed
        {
            debug_assert!(
                !self.insert_emit_bootstrap_bypass,
                "insertDecodeContractViolation: InsertGate Emit but decode bootstrap gate rejected"
            );
            if self.insert_emit_bootstrap_bypass {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc] insertDecodeContractViolation rtpTs={} bootstrapReject={:?}",
                    frame_rtp_timestamp,
                    encoded_frame.h264.bootstrap_reject_reason
                );
            }
            if self.nominal_continuation_hw_no_output_resets > 0
                && self.decoder_backend_is_hardware()
                && self.should_recover_nominal_continuation_no_output(
                    now_ms,
                    XbxVideoRecoveryState::Nominal,
                    encoded_frame.h264.bootstrap_reject_reason,
                )
            {
                let _ = self.fallback_hardware_backend_to_software(
                    now_ms,
                    "nominal-continuation-no-output",
                    "nominalContinuationNoOutputSoftFallback",
                    None,
                );
            }
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
        let target_time = encoded_frame.target_playout_instant;
        let rtp_timestamp = encoded_frame.rtp_timestamp;
        let is_keyframe = encoded_frame.is_keyframe;
        let budget = encoded_frame.budget;
        let frame_recovery_disposition = encoded_frame.frame_recovery_disposition;
        let frame_unrecoverable_reason = encoded_frame.frame_unrecoverable_reason.clone();
        let recovery_epoch_tag = encoded_frame.recovery_epoch_tag;
        let clean_anchor_commit_recovery_epoch = encoded_frame.clean_anchor_commit_recovery_epoch;
        let recovery_owner_rtp_timestamp = encoded_frame.recovery_owner_rtp_timestamp;
        let recovery_state_before_decode = self.recovery_state;
        let frame_nal_labels = encoded_frame.h264.nal_type_labels().join("|");
        let frame_bootstrap_reject_reason_kind = encoded_frame.h264.bootstrap_reject_reason;
        let frame_bootstrap_reject_reason = encoded_frame
            .h264
            .bootstrap_reject_reason
            .map(|reason| reason.as_str().to_string())
            .unwrap_or_else(|| "none".to_string());
        let frame_nal_count = encoded_frame.h264.nals.len();
        let frame_bootstrap_ready = encoded_frame.h264.bootstrap_ready;
        let frame_has_inband_sps = encoded_frame.h264.has_inband_sps;
        let frame_has_inband_pps = encoded_frame.h264.has_inband_pps;
        let frame_parameter_sets_changed = encoded_frame.h264.parameter_sets_changed;
        let frame_config_changed = encoded_frame.h264.config_changed;
        let bootstrap_config_change_idr = frame_is_keyframe
            && frame_bootstrap_ready
            && (frame_parameter_sets_changed || frame_config_changed);
        let frame_slice_headers_valid = encoded_frame.h264.slice_headers_valid;
        let frame_width = encoded_frame.width;
        let frame_height = encoded_frame.height;
        let frame_payload_bytes = encoded_frame.payload.len();
        let frame_payload_fingerprint = payload_fingerprint(encoded_frame.payload.as_ref());
        let frame_payload_prefix_hex = payload_prefix_hex(encoded_frame.payload.as_ref(), 24);
        if frame_parameter_sets_changed && frame_is_keyframe {
            crate::xbx_log_warn!(
                "[xbxengine][rtc] keyframe carries parameter-set change backend={} rtpTs={} size={}x{} bootstrapReady={} bootstrapReject={} hasInbandSps={} hasInbandPps={} nalCount={} nalTypes={}",
                self.decoder.backend_name(),
                frame_rtp_timestamp,
                encoded_frame.width,
                encoded_frame.height,
                frame_bootstrap_ready,
                frame_bootstrap_reject_reason,
                frame_has_inband_sps,
                frame_has_inband_pps,
                frame_nal_count,
                frame_nal_labels
            );
        }
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
                let transient_ffmpeg = is_transient_ffmpeg_decode_status(status);
                if should_count_toward_hardware_decode_fallback(status, frame_is_keyframe) {
                    self.record_hardware_decode_failure(now_ms, status);
                }
                self.backend_no_output_streak = 0;
                if transient_ffmpeg && !frame_is_keyframe {
                    self.last_transient_decode_error_at_ms = Some(now_ms);
                }
                let decode_detail = if transient_ffmpeg && !frame_is_keyframe {
                    "ffmpegInvalidData"
                } else {
                    "backendError"
                };
                self.record_decode_output_path_observation(
                    XbxDecodeOutputPathVerdict::BackendError,
                    decode_detail,
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
                if self.hardware_decode_failure_streak == 1 && !transient_ffmpeg {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc] hardware decode failed status={:?} err={error}",
                        status
                    );
                }
                if should_escalate_hardware_decode_fallback(
                    status,
                    frame_is_keyframe,
                    self.hardware_decode_failure_streak,
                ) {
                    self.record_remote_frame_capture_observation(
                        "backend-error",
                        &frame_nal_labels,
                        frame_nal_count,
                        frame_rtp_timestamp,
                        frame_is_keyframe,
                        frame_width,
                        frame_height,
                        frame_payload_bytes,
                        frame_payload_fingerprint,
                        &frame_payload_prefix_hex,
                        frame_has_inband_sps,
                        frame_has_inband_pps,
                        frame_bootstrap_ready,
                        Some(frame_bootstrap_reject_reason.clone()),
                        frame_parameter_sets_changed,
                        frame_config_changed,
                        frame_slice_headers_valid,
                        None,
                        None,
                        status,
                        Some(self.backend_no_output_streak),
                        Some(self.input_frames_since_last_decoded),
                        now_ms,
                    );
                    if self.decoder_backend_is_hardware() {
                        let _ = self.fallback_hardware_backend_to_software(
                            now_ms,
                            "backend-error",
                            "backendErrorSoftFallback",
                            status,
                        );
                    } else {
                        let _ = self.reset_decoder_with_failure(status, now_ms);
                    }
                }
                return None;
            }
        };
        let mut decoded_iter = decode_outcome.frames.into_iter();
        let Some(mut render_frame) = decoded_iter.next() else {
            let waiting_keyframe_bootstrap_no_output = matches!(
                recovery_state_before_decode,
                XbxVideoRecoveryState::WaitingKeyframe
            ) && frame_is_keyframe;
            if waiting_keyframe_bootstrap_no_output {
                self.arm_waiting_keyframe_continuation(now_ms);
            }
            self.backend_no_output_streak = self.backend_no_output_streak.saturating_add(1);
            if self.backend_no_output_streak >= CONTINUATION_NO_OUTPUT_KEYFRAME_HINT_THRESHOLD
                && matches!(
                    frame_bootstrap_reject_reason_kind,
                    Some(
                        H264BootstrapRejectReason::BootstrapMissingIdr
                            | H264BootstrapRejectReason::NonIdrVcl
                    )
                )
            {
                self.pending_receive_keyframe_hint_at_ms = Some(now_ms);
            }
            if self.decoder_backend_is_hardware() && self.latest_decoded_seq == 0 {
                self.first_hardware_no_output_at_ms.get_or_insert(now_ms);
            }
            let no_output_detail = self.classify_backend_no_output_detail(
                recovery_state_before_decode,
                continuation_gate_bypassed,
                waiting_keyframe_continuation_allowed,
                frame_is_keyframe,
                frame_parameter_sets_changed,
                frame_bootstrap_reject_reason_kind,
                recovery_epoch_tag,
                clean_anchor_commit_recovery_epoch,
                frame_recovery_disposition,
            );
            self.record_decode_output_path_observation(
                XbxDecodeOutputPathVerdict::BackendNoOutput,
                no_output_detail,
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
            if waiting_keyframe_bootstrap_no_output
                || self.backend_no_output_streak >= HARDWARE_NO_OUTPUT_SOFT_FALLBACK_THRESHOLD
            {
                self.record_remote_frame_capture_observation(
                    "backend-no-output",
                    &frame_nal_labels,
                    frame_nal_count,
                    frame_rtp_timestamp,
                    frame_is_keyframe,
                    frame_width,
                    frame_height,
                    frame_payload_bytes,
                    frame_payload_fingerprint,
                    &frame_payload_prefix_hex,
                    frame_has_inband_sps,
                    frame_has_inband_pps,
                    frame_bootstrap_ready,
                    Some(frame_bootstrap_reject_reason.clone()),
                    frame_parameter_sets_changed,
                    frame_config_changed,
                    frame_slice_headers_valid,
                    decode_outcome.send_packet_status,
                    decode_outcome.receive_frame_status,
                    None,
                    Some(self.backend_no_output_streak),
                    Some(self.input_frames_since_last_decoded),
                    now_ms,
                );
                crate::xbx_log_warn!(
                    "[xbxengine][rtc] decode backend produced no frame backend={} rtpTs={} keyframe={} recoveryState={} detail={} noOutputStreak={} inputSinceLastDecoded={} sendStatus={:?} receiveStatus={:?} bootstrapReady={} bootstrapReject={} hasInbandSps={} hasInbandPps={} parameterSetsChanged={} configChanged={} sliceHeadersValid={} nalCount={} nalTypes={}",
                    self.decoder.backend_name(),
                    frame_rtp_timestamp,
                    frame_is_keyframe,
                    recovery_state_before_decode.as_str(),
                    no_output_detail,
                    self.backend_no_output_streak,
                    self.input_frames_since_last_decoded,
                    decode_outcome.send_packet_status,
                    decode_outcome.receive_frame_status,
                    frame_bootstrap_ready,
                    frame_bootstrap_reject_reason,
                    frame_has_inband_sps,
                    frame_has_inband_pps,
                    frame_parameter_sets_changed,
                    frame_config_changed,
                    frame_slice_headers_valid,
                    frame_nal_count,
                    frame_nal_labels
                );
            }
            // bootstrap 关键帧常伴随 parameter_sets_changed；若已无输出且已打开 continuation 窗口，
            // 不要立刻 reset（reset 会清 continuation，导致后续 delta 无法再试解）。
            if frame_is_keyframe
                && frame_parameter_sets_changed
                && !waiting_keyframe_bootstrap_no_output
            {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc] keyframe no-output after parameter-set change, reset local decoder backend={} rtpTs={} noOutputStreak={} sendStatus={:?} receiveStatus={:?}",
                    self.decoder.backend_name(),
                    frame_rtp_timestamp,
                    self.backend_no_output_streak,
                    decode_outcome.send_packet_status,
                    decode_outcome.receive_frame_status
                );
                let _ = self.reset_decoder_with_failure(None, now_ms);
                return None;
            }
            if self.should_recover_recovering_continuation_no_output(
                now_ms,
                recovery_state_before_decode,
                frame_bootstrap_reject_reason_kind,
                recovery_epoch_tag,
                clean_anchor_commit_recovery_epoch,
                frame_recovery_disposition,
            ) {
                if self.decoder_backend_is_hardware() {
                    let _ = self.fallback_hardware_backend_to_software(
                        now_ms,
                        "recovering-continuation-no-output",
                        "recoveringContinuationNoOutputSoftFallback",
                        None,
                    );
                } else {
                    let _ = self.reset_decoder_with_failure_detail(
                        None,
                        now_ms,
                        "recoveringContinuationNoOutputReset",
                    );
                }
                return None;
            }
            if self.should_recover_nominal_continuation_no_output(
                now_ms,
                recovery_state_before_decode,
                frame_bootstrap_reject_reason_kind,
            ) {
                if self.decoder_backend_is_hardware()
                    && self.nominal_continuation_hw_no_output_resets == 0
                {
                    self.nominal_continuation_hw_no_output_resets = 1;
                    let _ = self.reset_decoder_with_failure_detail(
                        None,
                        now_ms,
                        "nominalContinuationNoOutputReset",
                    );
                } else if self.decoder_backend_is_hardware() {
                    let _ = self.fallback_hardware_backend_to_software(
                        now_ms,
                        "nominal-continuation-no-output",
                        "nominalContinuationNoOutputSoftFallback",
                        None,
                    );
                } else {
                    let _ = self.reset_decoder_with_failure_detail(
                        None,
                        now_ms,
                        "nominalContinuationNoOutputReset",
                    );
                }
                return None;
            }
            if self.should_rebuild_d3d11va_backend_after_no_output(
                recovery_state_before_decode,
                frame_is_keyframe,
            ) {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc] d3d11va backend-no-output reached rebuild threshold, force local rebuild backend={} rtpTs={} noOutputStreak={} rebuildAttempt={}",
                    self.decoder.backend_name(),
                    frame_rtp_timestamp,
                    self.backend_no_output_streak,
                    self.d3d11va_no_output_rebuild_attempts.saturating_add(1)
                );
                self.d3d11va_no_output_rebuild_attempts =
                    self.d3d11va_no_output_rebuild_attempts.saturating_add(1);
                let _ = self.reset_decoder_with_failure_detail(
                    None,
                    now_ms,
                    "d3d11vaBackendNoOutputRebuild",
                );
                return None;
            }
            if self.should_fallback_hardware_backend_after_no_output(
                recovery_state_before_decode,
                frame_is_keyframe,
                now_ms,
            ) {
                let _ = self.fallback_hardware_backend_to_software(
                    now_ms,
                    "backend-no-output",
                    "backendNoOutputSoftFallback",
                    None,
                );
            }
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
            XbxVideoRecoveryState::Recovering => {
                if !recovery_chain_unsettled(recovery_epoch_tag, frame_recovery_disposition) {
                    self.transition_recovery_state(
                        XbxVideoRecoveryState::Nominal,
                        XbxVideoRecoveryEvent::RecoverySettled,
                        "recoverySettled",
                        Some(rtp_timestamp as u64),
                        None,
                        now_ms,
                    );
                }
            }
            XbxVideoRecoveryState::Nominal => {}
        }
        self.reset_hardware_failure_streak();
        self.backend_no_output_streak = 0;
        self.first_hardware_no_output_at_ms = None;
        self.d3d11va_no_output_rebuild_attempts = 0;
        self.nominal_continuation_hw_no_output_resets = 0;
        self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
        self.last_decode_ok_time_ms = Some(now_ms);
        self.last_transient_decode_error_at_ms = None;
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
        if frame_is_keyframe && (frame_parameter_sets_changed || frame_config_changed) {
            self.record_remote_frame_capture_observation(
                "decoded-keyframe-config-change",
                &frame_nal_labels,
                frame_nal_count,
                frame_rtp_timestamp,
                frame_is_keyframe,
                frame_width,
                frame_height,
                frame_payload_bytes,
                frame_payload_fingerprint,
                &frame_payload_prefix_hex,
                frame_has_inband_sps,
                frame_has_inband_pps,
                frame_bootstrap_ready,
                Some(frame_bootstrap_reject_reason.clone()),
                frame_parameter_sets_changed,
                frame_config_changed,
                frame_slice_headers_valid,
                decode_outcome.send_packet_status,
                decode_outcome.receive_frame_status,
                None,
                Some(self.backend_no_output_streak),
                Some(self.input_frames_since_last_decoded),
                now_ms,
            );
        }
        self.input_frames_since_last_decoded = 0;
        render_frame.frame_seq = self.latest_decoded_seq;
        render_frame.rendered_at_ms = now_ms;
        render_frame.rtp_timestamp = Some(rtp_timestamp);
        render_frame.recovery_epoch_tag = recovery_epoch_tag;
        render_frame.recovery_owner_rtp_timestamp = recovery_owner_rtp_timestamp;
        render_frame.is_keyframe = is_keyframe;
        render_frame.frame_recovery_disposition = frame_recovery_disposition
            .render_label()
            .map(str::to_string);
        render_frame.frame_unrecoverable_reason = frame_unrecoverable_reason.clone();
        let presentation_value_role = if bootstrap_config_change_idr {
            crate::api::backend::XbxEnginePresentationValueRole::FreshAnchor
        } else {
            derive_presentation_value_role(
                clean_anchor_commit_recovery_epoch,
                recovery_epoch_tag,
                recovery_owner_rtp_timestamp,
                frame_recovery_disposition,
                frame_unrecoverable_reason.as_deref(),
                budget,
            )
        };
        render_frame.presentation_value_role = Some(presentation_value_role.as_str().to_string());
        let dropped_from_primary = self.enqueue_decoded_frame(DecodedFrame {
            pts: target_time,
            rtp_timestamp,
            is_keyframe,
            recovery_epoch_tag,
            recovery_owner_rtp_timestamp,
            clean_anchor_commit_recovery_epoch,
            presentation_value_role: Some(presentation_value_role),
            budget,
            frame_recovery_disposition,
            frame_unrecoverable_reason: frame_unrecoverable_reason.clone(),
            surface: render_frame,
        });
        for mut extra in decoded_iter {
            self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
            extra.frame_seq = self.latest_decoded_seq;
            extra.rendered_at_ms = now_ms;
            extra.rtp_timestamp = Some(rtp_timestamp);
            extra.recovery_epoch_tag = recovery_epoch_tag;
            extra.recovery_owner_rtp_timestamp = recovery_owner_rtp_timestamp;
            extra.is_keyframe = false;
            extra.frame_recovery_disposition = frame_recovery_disposition
                .render_label()
                .map(str::to_string);
            extra.frame_unrecoverable_reason = frame_unrecoverable_reason.clone();
            extra.presentation_value_role = Some(presentation_value_role.as_str().to_string());
            let _ = self.enqueue_decoded_frame(DecodedFrame {
                pts: target_time,
                rtp_timestamp,
                is_keyframe: false,
                recovery_epoch_tag,
                recovery_owner_rtp_timestamp,
                clean_anchor_commit_recovery_epoch: None,
                presentation_value_role: Some(presentation_value_role),
                budget,
                frame_recovery_disposition,
                frame_unrecoverable_reason: frame_unrecoverable_reason.clone(),
                surface: extra,
            });
        }
        dropped_from_primary
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

    fn record_decode_output_drop(&mut self, _detail: &'static str) {
        self.decoded_frame_drop_count = self.decoded_frame_drop_count.saturating_add(1);
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

    pub(crate) fn latest_remote_frame_capture_observation(
        &self,
    ) -> Option<&XbxRemoteFrameCaptureObservationSnapshot> {
        self.latest_remote_frame_capture_observation.as_ref()
    }

    pub(crate) fn latest_decode_candidate_decision(
        &self,
    ) -> Option<&XbxDecodeCandidateDecisionSnapshot> {
        self.latest_decode_candidate_decision.as_ref()
    }

    pub(crate) fn pop_decoded_frame(&mut self, _now_ms: f64) -> Option<DecodedFrame> {
        if self.decoded_inflight_current.is_none() {
            self.decoded_inflight_current = self.decoded_latest_candidate.take();
        }
        self.decoded_inflight_current.take()
    }

    pub(crate) fn has_decoded_frame(&self) -> bool {
        self.decoded_inflight_current.is_some() || self.decoded_latest_candidate.is_some()
    }

    #[allow(dead_code)]
    pub(crate) fn workload_snapshot(&self) -> XbxDecodeWorkloadSnapshot {
        let pending_output_queue_depth = self.decoded_output_mailbox_len();
        let state = if pending_output_queue_depth >= DECODE_OUTPUT_MAILBOX_CAPACITY {
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
        if self.should_enable_hard_backpressure() {
            XbxDecodeIngressDemand::PullOutputFirst
        } else {
            XbxDecodeIngressDemand::AcceptInput
        }
    }

    /// 解码输出邮箱已是 latest-only（`enqueue_decoded_frame` 会 supersede），不应再阻塞 ingress。
    /// 显示链背压只在 pacer/renderer/native 边界丢帧，避免「decode 30fps、present 10fps」时拖死解码。
    fn should_enable_hard_backpressure(&self) -> bool {
        false
    }

    #[cfg(test)]
    pub(crate) fn peek_decoded_frame(&self) -> Option<&DecodedFrame> {
        self.decoded_inflight_current
            .as_ref()
            .or(self.decoded_latest_candidate.as_ref())
    }

    pub(crate) fn decoded_frame_queue_len(&self) -> usize {
        self.decoded_output_mailbox_len()
    }

    /// 根据host cadence phase动态调整解码输出队列容量
    ///
    /// 策略：
    /// - Starved: 1帧（激进收紧，host消费过快）
    /// - Priming: 2帧（适度收紧，正在建立节奏）
    /// - Steady: 3帧（正常容量）
    /// - Idle/Unknown: 4帧（放宽，允许更多缓冲）
    #[cfg(test)]
    pub(crate) fn decoded_frame_queue_is_full(&self) -> bool {
        self.decoded_output_mailbox_len() >= DECODE_OUTPUT_MAILBOX_CAPACITY
    }

    #[cfg(test)]
    pub(crate) fn requeue_decoded_frame_front(&mut self, frame: DecodedFrame) {
        if self.decoded_inflight_current.is_none() {
            self.decoded_inflight_current = Some(frame);
            return;
        }
        let previous_inflight = self.decoded_inflight_current.replace(frame);
        if let Some(prev) = previous_inflight {
            let _ = self.enqueue_decoded_frame(prev);
        }
    }

    fn enqueue_decoded_frame(&mut self, mut frame: DecodedFrame) -> Option<DecodedFrame> {
        Self::normalize_decoded_surface_recovery_metadata(&mut frame);
        let incoming_frame_seq = frame.surface.frame_seq;
        let observed_at_ms = frame.surface.rendered_at_ms;
        if self.decoded_frame_is_stale(&frame, Instant::now()) {
            self.record_decode_output_drop("staleAfterDecode");
            self.record_decode_candidate_decision(
                XbxDecodeCandidateState::Backpressure,
                "drop",
                "staleAfterDecode",
                Some(incoming_frame_seq),
                None,
                observed_at_ms,
            );
            return Some(frame);
        }

        let Some(existing) = self.decoded_latest_candidate.take() else {
            self.decoded_latest_candidate = Some(frame);
            self.last_decode_latest_replace_at_ms = Some(observed_at_ms);
            if matches!(
                self.decode_candidate_state,
                XbxDecodeCandidateState::Backpressure
            ) {
                self.record_decode_candidate_decision(
                    XbxDecodeCandidateState::Nominal,
                    "accept",
                    "mailboxRecovered",
                    Some(incoming_frame_seq),
                    None,
                    observed_at_ms,
                );
            }
            return None;
        };

        if self.should_protect_anchor_candidate_from_incoming_burst(
            &frame,
            &existing,
            observed_at_ms,
        ) {
            self.decoded_latest_candidate = Some(existing);
            self.record_decode_output_drop("coalescedAfterDecode");
            self.record_decode_candidate_decision(
                XbxDecodeCandidateState::Backpressure,
                "drop",
                "coalescedAfterDecode",
                Some(incoming_frame_seq),
                None,
                observed_at_ms,
            );
            return Some(frame);
        }

        let (keep, drop) = if Self::compare_decoded_candidate_value(&frame, &existing) >= 0 {
            (frame, existing)
        } else {
            (existing, frame)
        };
        let replacement_decision = Some(
            crate::api::backend::XbxEngineReplacementDecisionObservation {
                dropped_frame_seq: Some(drop.surface.frame_seq),
                dropped_rtp_timestamp: Some(drop.rtp_timestamp),
                dropped_presentation_value_role: Some(
                    decoded_presentation_value_role(&drop).as_str().to_string(),
                ),
                kept_frame_seq: Some(keep.surface.frame_seq),
                kept_rtp_timestamp: Some(keep.rtp_timestamp),
                kept_presentation_value_role: Some(
                    decoded_presentation_value_role(&keep).as_str().to_string(),
                ),
                same_recovery_epoch: Some(keep.recovery_epoch_tag == drop.recovery_epoch_tag),
                same_recovery_owner_chain: Some(
                    keep.recovery_epoch_tag == drop.recovery_epoch_tag
                        && keep.recovery_owner_rtp_timestamp == drop.recovery_owner_rtp_timestamp,
                ),
                supersede_reason: Some(Self::decoded_supersede_reason(&keep, &drop).to_string()),
            },
        );
        self.decoded_latest_candidate = Some(keep);
        self.last_decode_latest_replace_at_ms = Some(observed_at_ms);
        self.record_decode_output_drop("supersededAfterDecode");
        self.record_decode_candidate_decision(
            XbxDecodeCandidateState::Backpressure,
            "drop",
            "supersededAfterDecode",
            Some(drop.surface.frame_seq),
            replacement_decision,
            observed_at_ms,
        );
        Some(drop)
    }

    fn normalize_decoded_surface_recovery_metadata(frame: &mut DecodedFrame) {
        frame.surface.frame_recovery_disposition = frame
            .frame_recovery_disposition
            .render_label()
            .map(str::to_string);
        frame.surface.frame_unrecoverable_reason = frame.frame_unrecoverable_reason.clone();
    }

    fn decoded_frame_is_stale(&self, frame: &DecodedFrame, now: Instant) -> bool {
        now > frame.pts + self.decoded_frame_stale_slack(frame)
    }

    fn decoded_frame_stale_slack(&self, frame: &DecodedFrame) -> Duration {
        let base_millis = match frame.budget.recovery_value_tier() {
            "anchor" => DECODE_QUEUE_STALE_SLACK_ANCHOR_MS,
            "supply" => DECODE_QUEUE_STALE_SLACK_SUPPLY_MS,
            _ => DECODE_QUEUE_STALE_SLACK_DISPOSABLE_MS,
        };
        let cadence_scaled_millis = self
            .decoded_mailbox_frame_interval_hint_ms(frame)
            .map(|interval_ms| match frame.budget.recovery_value_tier() {
                "anchor" => interval_ms
                    .saturating_mul(2)
                    .saturating_add(DECODE_QUEUE_STALE_SLACK_GUARD_MS),
                "supply" => interval_ms
                    .saturating_mul(3)
                    .saturating_div(2)
                    .saturating_add(DECODE_QUEUE_STALE_SLACK_GUARD_MS),
                _ => interval_ms.saturating_add(DECODE_QUEUE_STALE_SLACK_GUARD_MS),
            })
            .unwrap_or(base_millis);
        let recovery_bonus_millis = if Self::decoded_frame_uses_recovery_window(frame) {
            DECODE_QUEUE_STALE_SLACK_RECOVERY_BONUS_MS
        } else {
            0
        };
        Duration::from_millis(base_millis.max(cadence_scaled_millis) + recovery_bonus_millis)
    }

    fn decoded_mailbox_frame_interval_hint_ms(&self, frame: &DecodedFrame) -> Option<u64> {
        let mut hint_ms = None;
        for existing in [
            self.decoded_inflight_current.as_ref(),
            self.decoded_latest_candidate.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let delta = if frame.pts >= existing.pts {
                frame.pts.duration_since(existing.pts)
            } else {
                existing.pts.duration_since(frame.pts)
            };
            let delta_ms = delta.as_millis() as u64;
            if (8..=100).contains(&delta_ms) {
                hint_ms = Some(hint_ms.map_or(delta_ms, |current: u64| current.min(delta_ms)));
            }
        }
        hint_ms
    }

    fn decoded_frame_uses_recovery_window(frame: &DecodedFrame) -> bool {
        matches!(
            frame.budget.window_source,
            FrameBudgetWindowSource::Recovery
        )
    }

    fn decoded_output_mailbox_len(&self) -> usize {
        usize::from(self.decoded_inflight_current.is_some())
            + usize::from(self.decoded_latest_candidate.is_some())
    }

    fn clear_decoded_output_mailbox(&mut self) {
        self.decoded_inflight_current = None;
        self.decoded_latest_candidate = None;
    }

    fn compare_decoded_candidate_value(a: &DecodedFrame, b: &DecodedFrame) -> i32 {
        let compare_result = crate::api::backend::compare_latest_only_frame_meta(
            &crate::api::backend::XbxEngineLatestOnlyFrameMeta {
                presentation_value_role: decoded_presentation_value_role(a),
                recovery_epoch_tag: a
                    .recovery_epoch_tag
                    .or(a.clean_anchor_commit_recovery_epoch),
                recovery_owner_rtp_timestamp: a.recovery_owner_rtp_timestamp,
                rtp_timestamp: Some(a.rtp_timestamp),
                frame_seq: Some(a.surface.frame_seq),
                rendered_at_ms: a.surface.rendered_at_ms,
                owner_preference_active: matches!(
                    decoded_presentation_value_role(a),
                    crate::api::backend::XbxEnginePresentationValueRole::FreshAnchor
                        | crate::api::backend::XbxEnginePresentationValueRole::RecoveryContinuation
                ),
                value_rank: decoded_presentation_value_role(a).rank(),
            },
            &crate::api::backend::XbxEngineLatestOnlyFrameMeta {
                presentation_value_role: decoded_presentation_value_role(b),
                recovery_epoch_tag: b
                    .recovery_epoch_tag
                    .or(b.clean_anchor_commit_recovery_epoch),
                recovery_owner_rtp_timestamp: b.recovery_owner_rtp_timestamp,
                rtp_timestamp: Some(b.rtp_timestamp),
                frame_seq: Some(b.surface.frame_seq),
                rendered_at_ms: b.surface.rendered_at_ms,
                owner_preference_active: matches!(
                    decoded_presentation_value_role(b),
                    crate::api::backend::XbxEnginePresentationValueRole::FreshAnchor
                        | crate::api::backend::XbxEnginePresentationValueRole::RecoveryContinuation
                ),
                value_rank: decoded_presentation_value_role(b).rank(),
            },
        );
        if compare_result != 0 {
            return compare_result;
        }
        if a.pts > b.pts {
            1
        } else if a.pts < b.pts {
            -1
        } else {
            0
        }
    }

    fn decoded_supersede_reason(keep: &DecodedFrame, drop: &DecodedFrame) -> &'static str {
        if decoded_presentation_value_role(keep).rank()
            > decoded_presentation_value_role(drop).rank()
        {
            return "higherRole";
        }
        if keep.recovery_epoch_tag == drop.recovery_epoch_tag
            && keep.recovery_owner_rtp_timestamp == drop.recovery_owner_rtp_timestamp
            && keep.surface.frame_seq > drop.surface.frame_seq
        {
            return "newerWithinSameRecoveryChain";
        }
        if matches!(
            decoded_presentation_value_role(drop),
            crate::api::backend::XbxEnginePresentationValueRole::FreshAnchor
        ) {
            return "anchorProtection";
        }
        "newerWithinSameRole"
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

    fn should_fallback_hardware_backend_after_no_output(
        &self,
        recovery_state_before_decode: XbxVideoRecoveryState,
        frame_is_keyframe: bool,
        now_ms: f64,
    ) -> bool {
        self.decoder_backend_is_hardware()
            && self.latest_decoded_seq == 0
            && (frame_is_keyframe
                || matches!(
                    recovery_state_before_decode,
                    XbxVideoRecoveryState::WaitingKeyframe
                ))
            && (self.backend_no_output_streak >= HARDWARE_NO_OUTPUT_SOFT_FALLBACK_THRESHOLD
                || self.first_hardware_no_output_at_ms.is_some_and(|t0| {
                    (now_ms - t0).max(0.0) >= HARDWARE_NO_OUTPUT_SOFT_FALLBACK_WINDOW_MS
                }))
    }

    fn should_rebuild_d3d11va_backend_after_no_output(
        &self,
        recovery_state_before_decode: XbxVideoRecoveryState,
        frame_is_keyframe: bool,
    ) -> bool {
        cfg!(target_os = "windows")
            && self.decoder_backend_is_d3d11va()
            && self.d3d11va_no_output_rebuild_attempts < D3D11VA_NO_OUTPUT_MAX_REBUILD_ATTEMPTS
            && self.backend_no_output_streak >= D3D11VA_NO_OUTPUT_REBUILD_THRESHOLD
            && (frame_is_keyframe
                || matches!(
                    recovery_state_before_decode,
                    XbxVideoRecoveryState::WaitingKeyframe
                ))
    }

    fn should_suppress_repeat_waiting_keyframe_decoder_reset(&self, now_ms: f64) -> bool {
        if !matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe) {
            return false;
        }
        let Some(reset_at_ms) = self.latest_decoder_reset_time_ms else {
            return false;
        };
        if self
            .last_decoder_reset_success_edge_at_ms
            .is_some_and(|success_at_ms| success_at_ms > reset_at_ms)
            || self
                .last_decode_ok_time_ms
                .is_some_and(|ok_at_ms| ok_at_ms > reset_at_ms)
        {
            return false;
        }
        let _ = now_ms;
        true
    }

    fn should_recover_nominal_continuation_no_output(
        &self,
        now_ms: f64,
        recovery_state_before_decode: XbxVideoRecoveryState,
        frame_bootstrap_reject_reason: Option<H264BootstrapRejectReason>,
    ) -> bool {
        if matches!(
            recovery_state_before_decode,
            XbxVideoRecoveryState::Recovering | XbxVideoRecoveryState::WaitingKeyframe
        ) {
            return false;
        }
        if self
            .last_transient_decode_error_at_ms
            .is_some_and(|at| (now_ms - at).max(0.0) < TRANSIENT_DECODE_ERROR_RESET_COALESCE_MS)
        {
            return false;
        }
        if matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe) {
            return false;
        }
        matches!(recovery_state_before_decode, XbxVideoRecoveryState::Nominal)
            && self.latest_decoded_seq > 0
            && matches!(
                frame_bootstrap_reject_reason,
                Some(
                    H264BootstrapRejectReason::BootstrapMissingIdr
                        | H264BootstrapRejectReason::NonIdrVcl
                )
            )
            && self.backend_no_output_streak >= NOMINAL_CONTINUATION_NO_OUTPUT_RECOVERY_THRESHOLD
            && self.input_frames_since_last_decoded
                >= NOMINAL_CONTINUATION_NO_OUTPUT_RECOVERY_THRESHOLD
    }

    fn should_recover_recovering_continuation_no_output(
        &self,
        now_ms: f64,
        recovery_state_before_decode: XbxVideoRecoveryState,
        frame_bootstrap_reject_reason: Option<H264BootstrapRejectReason>,
        recovery_epoch_tag: Option<u64>,
        _clean_anchor_commit_recovery_epoch: Option<u64>,
        frame_recovery_disposition: crate::media::video::types::FrameRecoveryDisposition,
    ) -> bool {
        if self
            .last_transient_decode_error_at_ms
            .is_some_and(|at| (now_ms - at).max(0.0) < TRANSIENT_DECODE_ERROR_RESET_COALESCE_MS)
        {
            return false;
        }
        matches!(
            recovery_state_before_decode,
            XbxVideoRecoveryState::Recovering
        ) && self.latest_decoded_seq > 0
            && recovery_chain_unsettled(recovery_epoch_tag, frame_recovery_disposition)
            && matches!(
                frame_bootstrap_reject_reason,
                Some(
                    H264BootstrapRejectReason::BootstrapMissingIdr
                        | H264BootstrapRejectReason::NonIdrVcl
                )
            )
            && self.backend_no_output_streak >= RECOVERING_CONTINUATION_NO_OUTPUT_RECOVERY_THRESHOLD
            && self.input_frames_since_last_decoded
                >= RECOVERING_CONTINUATION_NO_OUTPUT_RECOVERY_THRESHOLD
    }

    fn reset_decoder_with_failure(
        &mut self,
        status: Option<i32>,
        now_ms: f64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.reset_decoder_with_failure_detail(status, now_ms, "backendFailureReset")
    }

    fn reset_decoder_with_failure_detail(
        &mut self,
        status: Option<i32>,
        now_ms: f64,
        transition_detail: &'static str,
    ) -> Result<(), XbxEngineRuntimeError> {
        if matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe)
            && self.should_suppress_repeat_waiting_keyframe_decoder_reset(now_ms)
        {
            return Ok(());
        }
        self.clear_decoded_output_mailbox();
        self.last_decode_ok_time_ms = None;
        self.reset_decoder_backend(now_ms);
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.reset_hardware_failure_streak();
        self.backend_no_output_streak = 0;
        self.input_frames_since_last_decoded = 0;
        self.first_hardware_no_output_at_ms = None;
        self.d3d11va_no_output_rebuild_attempts = 0;
        self.clear_waiting_keyframe_continuation();
        let continuation_unstick =
            self.timed_fallback_displayed_idr_bypass || self.insert_emit_bootstrap_bypass;
        if matches!(
            transition_detail,
            "nominalContinuationNoOutputReset" | "recoveringContinuationNoOutputReset"
        ) && !continuation_unstick
        {
            self.pending_receive_keyframe_hint_at_ms = Some(now_ms);
        }
        let next_state = if continuation_unstick {
            self.waiting_keyframe_continuation_deadline_ms =
                Some(now_ms + TIMED_FALLBACK_DISPLAYED_IDR_CONTINUATION_WINDOW_MS);
            self.waiting_keyframe_continuation_frames_left =
                TIMED_FALLBACK_DISPLAYED_IDR_CONTINUATION_MAX_FRAMES;
            XbxVideoRecoveryState::Recovering
        } else {
            XbxVideoRecoveryState::WaitingKeyframe
        };
        self.transition_recovery_state(
            next_state,
            XbxVideoRecoveryEvent::BackendFailureEscalated,
            transition_detail,
            None,
            status,
            now_ms,
        );
        Ok(())
    }

    fn fallback_hardware_backend_to_software(
        &mut self,
        now_ms: f64,
        fallback_reason: &'static str,
        transition_detail: &'static str,
        status: Option<i32>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let previous_backend_name = self.decoder.backend_name().to_string();
        let (decoder, probe) = (self.software_decoder_factory)();
        if probe.selected_backend_name == "noop" {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineSoftwareDecoderFallbackUnavailable:reason={fallback_reason}"
            )));
        }
        self.clear_decoded_output_mailbox();
        self.last_decode_ok_time_ms = None;
        self.decoder = decoder;
        let next_observation_id = self
            .latest_decoder_probe
            .as_ref()
            .map(|probe| probe.observation_id.saturating_add(1))
            .unwrap_or(1);
        let mut fallback_summary = probe.fallback_summary.unwrap_or_default();
        if !fallback_summary.is_empty() {
            fallback_summary.push_str(" -> ");
        }
        fallback_summary.push_str(&format!(
            "{previous_backend_name}(hardware/{fallback_reason})"
        ));
        self.latest_decoder_probe = Some(Self::build_decoder_probe_snapshot(
            next_observation_id,
            XbxVideoDecoderProbeSummary {
                fallback_count: probe.fallback_count.saturating_add(1),
                fallback_summary: Some(fallback_summary),
                ..probe
            },
            now_ms,
        ));
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.last_decoder_reset_success_edge_at_ms = None;
        self.reset_hardware_failure_streak();
        self.backend_no_output_streak = 0;
        self.input_frames_since_last_decoded = 0;
        self.first_hardware_no_output_at_ms = None;
        self.d3d11va_no_output_rebuild_attempts = 0;
        self.clear_waiting_keyframe_continuation();
        self.transition_recovery_state(
            XbxVideoRecoveryState::WaitingKeyframe,
            XbxVideoRecoveryEvent::BackendFailureEscalated,
            transition_detail,
            None,
            status,
            now_ms,
        );
        crate::xbx_log_warn!(
            "[xbxengine][rtc] hardware decoder runtime fallback to software backend previous_backend={} reason={} status={:?}",
            previous_backend_name,
            fallback_reason,
            status
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

    fn decoder_backend_is_hardware(&self) -> bool {
        matches!(
            self.decoder.backend_name(),
            "ffmpeg-videotoolbox" | "ffmpeg-d3d11va"
        )
    }

    fn decoder_backend_is_d3d11va(&self) -> bool {
        self.decoder.backend_name() == "ffmpeg-d3d11va"
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
        replacement_decision: Option<crate::api::backend::XbxEngineReplacementDecisionObservation>,
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
            replacement_decision,
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

    fn classify_backend_no_output_detail(
        &self,
        recovery_state_before_decode: XbxVideoRecoveryState,
        continuation_gate_bypassed: bool,
        waiting_keyframe_continuation_allowed: bool,
        frame_is_keyframe: bool,
        frame_parameter_sets_changed: bool,
        frame_bootstrap_reject_reason: Option<H264BootstrapRejectReason>,
        recovery_epoch_tag: Option<u64>,
        _clean_anchor_commit_recovery_epoch: Option<u64>,
        frame_recovery_disposition: crate::media::video::types::FrameRecoveryDisposition,
    ) -> &'static str {
        if continuation_gate_bypassed {
            "backendNoOutputAfterContinuationBypass"
        } else if matches!(
            recovery_state_before_decode,
            XbxVideoRecoveryState::WaitingKeyframe
        ) && frame_is_keyframe
        {
            "backendNoOutputAfterBootstrapKeyframe"
        } else if matches!(
            recovery_state_before_decode,
            XbxVideoRecoveryState::WaitingKeyframe
        ) && waiting_keyframe_continuation_allowed
        {
            "backendNoOutputAfterWaitingKeyframeContinuation"
        } else if matches!(
            recovery_state_before_decode,
            XbxVideoRecoveryState::Recovering
        ) && recovery_chain_unsettled(recovery_epoch_tag, frame_recovery_disposition)
            && matches!(
                frame_bootstrap_reject_reason,
                Some(
                    H264BootstrapRejectReason::BootstrapMissingIdr
                        | H264BootstrapRejectReason::NonIdrVcl
                )
            )
        {
            "backendNoOutputAfterRecoveringContinuation"
        } else if matches!(recovery_state_before_decode, XbxVideoRecoveryState::Nominal)
            && self.latest_decoded_seq > 0
            && matches!(
                frame_bootstrap_reject_reason,
                Some(
                    H264BootstrapRejectReason::BootstrapMissingIdr
                        | H264BootstrapRejectReason::NonIdrVcl
                )
            )
        {
            "backendNoOutputAfterNominalContinuation"
        } else if frame_is_keyframe && frame_parameter_sets_changed {
            "backendNoOutputAfterConfigChangeKeyframe"
        } else {
            "backendNoOutput"
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_remote_frame_capture_observation(
        &mut self,
        trigger: &'static str,
        frame_nal_labels: &str,
        frame_nal_count: usize,
        frame_rtp_timestamp: u32,
        is_keyframe: bool,
        width: u32,
        height: u32,
        payload_bytes: usize,
        payload_fingerprint: u64,
        payload_prefix_hex: &str,
        has_inband_sps: bool,
        has_inband_pps: bool,
        bootstrap_ready: bool,
        bootstrap_reject_reason: Option<String>,
        parameter_sets_changed: bool,
        config_changed: bool,
        slice_headers_valid: bool,
        send_packet_status: Option<i32>,
        receive_frame_status: Option<i32>,
        status: Option<i32>,
        backend_no_output_streak: Option<u32>,
        input_frames_since_last_decoded: Option<u32>,
        observed_at_ms: f64,
    ) {
        self.remote_frame_capture_observation_id =
            self.remote_frame_capture_observation_id.saturating_add(1);
        self.latest_remote_frame_capture_observation =
            Some(XbxRemoteFrameCaptureObservationSnapshot {
                observation_id: self.remote_frame_capture_observation_id,
                trigger,
                backend_name: self.decoder.backend_name().to_string(),
                frame_rtp_timestamp,
                is_keyframe,
                width,
                height,
                payload_bytes,
                payload_fingerprint,
                payload_prefix_hex: payload_prefix_hex.to_string(),
                nal_types: if frame_nal_labels.is_empty() {
                    Vec::new()
                } else {
                    frame_nal_labels
                        .split('|')
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                },
                nal_count: frame_nal_count as u16,
                has_inband_sps,
                has_inband_pps,
                bootstrap_ready,
                bootstrap_reject_reason,
                parameter_sets_changed,
                config_changed,
                slice_headers_valid,
                send_packet_status,
                receive_frame_status,
                status,
                backend_no_output_streak,
                input_frames_since_last_decoded,
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
        if !encoded_frame.is_keyframe
            && matches!(
                encoded_frame.h264.bootstrap_reject_reason,
                Some(
                    H264BootstrapRejectReason::BootstrapMissingIdr
                        | H264BootstrapRejectReason::NonIdrVcl
                )
            )
        {
            return false;
        }
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
        Self::new_for_test_with_factories(
            min_delay_ms,
            max_delay_ms,
            decoder,
            Box::new(|| {
                panic!("test decoder factory was not configured for decoder reset path");
            }),
            Box::new(|| {
                panic!(
                    "test software decoder factory was not configured for software fallback path"
                );
            }),
        )
    }

    fn new_for_test_with_factory(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxVideoDecoderBackend>,
        decoder_factory: XbxVideoDecoderFactory,
    ) -> Self {
        Self::new_for_test_with_factories(
            min_delay_ms,
            max_delay_ms,
            decoder,
            decoder_factory,
            Box::new(|| {
                panic!(
                    "test software decoder factory was not configured for software fallback path"
                );
            }),
        )
    }

    fn new_for_test_with_factories(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxVideoDecoderBackend>,
        decoder_factory: XbxVideoDecoderFactory,
        software_decoder_factory: XbxVideoDecoderFactory,
    ) -> Self {
        let _ = (min_delay_ms, max_delay_ms);
        Self {
            decoder,
            decoder_factory,
            software_decoder_factory,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_inflight_current: None,
            decoded_latest_candidate: None,
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
            latest_remote_frame_capture_observation: None,
            remote_frame_capture_observation_id: 0,
            backend_no_output_streak: 0,
            input_frames_since_last_decoded: 0,
            first_hardware_no_output_at_ms: None,
            waiting_keyframe_continuation_deadline_ms: None,
            waiting_keyframe_continuation_frames_left: 0,
            decode_candidate_state: XbxDecodeCandidateState::Nominal,
            latest_decode_candidate_decision: None,
            decode_candidate_decision_id: 0,
            d3d11va_no_output_rebuild_attempts: 0,
            nominal_continuation_hw_no_output_resets: 0,
            last_decode_latest_replace_at_ms: None,
            mailbox_present_cadence_interval_ms:
                crate::media::video::present_cadence::PRESENT_CADENCE_INTERVAL_FALLBACK_MS,
            timed_fallback_displayed_idr_bypass: false,
            insert_emit_bootstrap_bypass: false,
            pending_receive_keyframe_hint_at_ms: None,
            last_transient_decode_error_at_ms: None,
        }
    }

    pub(crate) fn enqueue_decoded_frame_for_test(&mut self, frame: XbxRenderFrame) {
        self.enqueue_decoded_frame_with_budget_for_test(
            frame,
            crate::media::video::ingress::budget::FrameBudgetContext::default(),
        );
    }

    pub(crate) fn enqueue_decoded_frame_with_budget_for_test(
        &mut self,
        frame: XbxRenderFrame,
        budget: crate::media::video::ingress::budget::FrameBudgetContext,
    ) {
        let _ = self.enqueue_decoded_frame(DecodedFrame {
            pts: std::time::Instant::now(),
            rtp_timestamp: frame.frame_seq as u32,
            is_keyframe: frame.is_keyframe,
            recovery_epoch_tag: frame.recovery_epoch_tag,
            recovery_owner_rtp_timestamp: frame.recovery_owner_rtp_timestamp,
            clean_anchor_commit_recovery_epoch: None,
            presentation_value_role: None,
            budget,
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: frame,
        });
    }

    #[cfg(test)]
    pub(crate) fn enqueue_decoded_frame_with_clean_anchor_epoch_for_test(
        &mut self,
        frame: XbxRenderFrame,
        clean_anchor_commit_recovery_epoch: Option<u64>,
    ) {
        let _ = self.enqueue_decoded_frame(DecodedFrame {
            pts: std::time::Instant::now(),
            rtp_timestamp: frame.frame_seq as u32,
            is_keyframe: frame.is_keyframe,
            recovery_epoch_tag: frame.recovery_epoch_tag,
            recovery_owner_rtp_timestamp: frame.recovery_owner_rtp_timestamp,
            clean_anchor_commit_recovery_epoch,
            presentation_value_role: None,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: frame,
        });
    }

    #[cfg(test)]
    pub(crate) fn enqueue_decoded_frame_with_clean_anchor_epoch_and_pts_for_test(
        &mut self,
        frame: XbxRenderFrame,
        clean_anchor_commit_recovery_epoch: Option<u64>,
        pts: std::time::Instant,
    ) {
        let _ = self.enqueue_decoded_frame(DecodedFrame {
            pts,
            rtp_timestamp: frame.frame_seq as u32,
            is_keyframe: frame.is_keyframe,
            recovery_epoch_tag: frame.recovery_epoch_tag,
            recovery_owner_rtp_timestamp: frame.recovery_owner_rtp_timestamp,
            clean_anchor_commit_recovery_epoch,
            presentation_value_role: None,
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

/// FFmpeg `AVERROR_INVALIDDATA` / EAGAIN：常见于 PPS 未就绪或 drain 时序，不是 VideoToolbox 硬故障。
fn is_transient_ffmpeg_decode_status(status: Option<i32>) -> bool {
    matches!(
        status,
        Some(code) if code == super::backend_ffmpeg::av_err_invaliddata()
            || code == super::backend_ffmpeg::av_err_eagain()
    )
}

fn should_count_toward_hardware_decode_fallback(
    status: Option<i32>,
    frame_is_keyframe: bool,
) -> bool {
    if is_transient_ffmpeg_decode_status(status) && !frame_is_keyframe {
        return false;
    }
    should_force_recovery_keyframe(status) || frame_is_keyframe
}

/// 禁止在 delta + AVERROR_INVALIDDATA 上误触发硬解→软解 fallback（会焊死 waiting-keyframe）。
fn should_escalate_hardware_decode_fallback(
    status: Option<i32>,
    frame_is_keyframe: bool,
    hardware_decode_failure_streak: u32,
) -> bool {
    if should_force_recovery_keyframe(status) {
        return true;
    }
    if is_transient_ffmpeg_decode_status(status) {
        return false;
    }
    frame_is_keyframe && hardware_decode_failure_streak >= 3
}

fn payload_fingerprint(payload: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    hasher.finish()
}

fn payload_prefix_hex(payload: &[u8], max_bytes: usize) -> String {
    let mut out = String::new();
    for byte in payload.iter().take(max_bytes) {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{byte:02x}"));
    }
    out
}
const K_VT_VIDEO_DECODER_BAD_DATA_ERR: i32 = -12909;
const K_VT_VIDEO_DECODER_REFERENCE_MISSING_ERR: i32 = -17694;

#[cfg(test)]
#[path = "video_decode.test.rs"]
mod tests;
