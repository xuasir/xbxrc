use super::{build_sample_builder, now_ms_f64, UINT16SIZE_HALF};
use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264BootstrapRejectReason};
use base64::Engine as _;
use bytes::Bytes;

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::{AssembledVideoFrame, FrameValue, VideoCodec};
use crate::transport::rtc::stream::adapter_types::{
    FrameSource, TransportAdmissionObservation, TransportLossObservation, TransportObservation,
    TransportObservationSource,
};
use crate::transport::rtc::stream::packet_types::{RtcVideoIngressKind, RtcVideoRepairMetadata};
use crate::transport::rtc::stream::video_source::{
    RtcVideoFrameSource, RtcVideoTransportObservationSource,
};

use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState,
    XbxEngineH264InspectionObservation, XbxEngineVideoRtxReinjectObservation,
};
use xbxengine_protocol::XbxEngineTargetTypeDto;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecoveryKeyframeAction {
    Submit,
    DropAndRequestKeyframe,
    TriggerWaitKeyframe,
    WaitKeyframe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InspectionAdmission {
    Accept,
    AwaitRecoveryKeyframe,
}

pub(super) fn resolve_inspection_admission(
    inspection: &H264AccessUnitInspection,
) -> InspectionAdmission {
    if !inspection.slice_headers_valid {
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    if inspection.bootstrap_ready || inspection.delta_continuation_ready() {
        return InspectionAdmission::Accept;
    }

    InspectionAdmission::AwaitRecoveryKeyframe
}

fn inspection_bootstrap_reason(inspection: &H264AccessUnitInspection) -> &'static str {
    match inspection.bootstrap_reject_reason {
        Some(H264BootstrapRejectReason::NoVcl) => "inspectionRejectNoVcl",
        Some(H264BootstrapRejectReason::MissingSps) => "bootstrapMissingSps",
        Some(H264BootstrapRejectReason::MissingPps) => "bootstrapMissingPps",
        Some(H264BootstrapRejectReason::NonIdrVcl) => "inspectionRejectNonIdrVcl",
        Some(H264BootstrapRejectReason::InvalidSliceHeader) => "inspectionRejectInvalidSliceHeader",
        None if !inspection.slice_headers_valid => "inspectionRejectInvalidSliceHeader",
        None => "inspectionRejectUnknown",
    }
}

pub(super) fn resolve_recovery_keyframe_action(
    waiting_for_recovery_keyframe: bool,
    sample_loss_burst_count: u8,
    media_dropped_packets: u16,
    is_keyframe: bool,
    allow_soft_reentry_submit: bool,
) -> (bool, RecoveryKeyframeAction) {
    // 带丢包的 keyframe/reference 不能继续喂给解码器，否则很容易把本地参考链喂脏，
    // 在 macOS 上会直接放大成 VideoToolbox 连续 bad-data 回调。
    if is_keyframe && media_dropped_packets > 0 {
        return (true, RecoveryKeyframeAction::TriggerWaitKeyframe);
    }

    if is_keyframe {
        return (false, RecoveryKeyframeAction::Submit);
    }

    if media_dropped_packets > 0 {
        if sample_loss_burst_count >= 2 {
            return (true, RecoveryKeyframeAction::TriggerWaitKeyframe);
        }
        return (false, RecoveryKeyframeAction::DropAndRequestKeyframe);
    }

    if waiting_for_recovery_keyframe {
        if allow_soft_reentry_submit {
            // clean anchor 后的短窗内，健康 delta 只要还能安全提交，就别继续把链路拖回 recovering。
            return (false, RecoveryKeyframeAction::Submit);
        }
        return (true, RecoveryKeyframeAction::WaitKeyframe);
    }

    (false, RecoveryKeyframeAction::Submit)
}

pub(super) fn detect_forward_gap(
    last_highest_rtp_sequence: Option<u16>,
    sequence: u16,
) -> (Option<u16>, Option<(u16, u16)>) {
    let Some(last_highest) = last_highest_rtp_sequence else {
        return (Some(sequence), None);
    };
    let diff = sequence.wrapping_sub(last_highest);
    if diff == 0 {
        return (Some(last_highest), None);
    }
    if diff < UINT16SIZE_HALF {
        if diff > 1 {
            return (
                Some(sequence),
                Some((last_highest.wrapping_add(1), sequence)),
            );
        }
        return (Some(sequence), None);
    }

    (Some(last_highest), None)
}

impl RtcVideoFrameSource {
    fn maybe_seed_h264_bootstrap_from_remote_answer(&self) {
        if self.h264_inspector.committed_sps_present()
            && self.h264_inspector.committed_pps_present()
        {
            return;
        }

        let sprop_parameter_sets = self
            .runtime_stats
            .read(|stats| {
                stats
                    .latest_remote_answer_observation
                    .as_ref()
                    .and_then(|observation| {
                        observation.selected_video_h264_sprop_parameter_sets.clone()
                    })
            })
            .flatten();
        let Some(sprop_parameter_sets) = sprop_parameter_sets else {
            return;
        };
        let [sps_b64, pps_b64, ..] = sprop_parameter_sets.as_slice() else {
            return;
        };

        let decode_engine = &base64::engine::general_purpose::STANDARD;
        let Ok(sps) = decode_engine.decode(sps_b64) else {
            crate::xbx_log_warn!("[RtcVideoFrameSource] failed to decode remote answer sprop SPS");
            return;
        };
        let Ok(pps) = decode_engine.decode(pps_b64) else {
            crate::xbx_log_warn!("[RtcVideoFrameSource] failed to decode remote answer sprop PPS");
            return;
        };

        match self
            .h264_inspector
            .seed_committed_parameter_sets_if_absent(&sps, &pps)
        {
            Ok(true) => {
                crate::xbx_log_info!(
                    "[RtcVideoFrameSource] seeded H264 bootstrap from remote answer sprop parameter sets"
                );
            }
            Ok(false) => {}
            Err(error) => {
                crate::xbx_log_warn!(
                    "[RtcVideoFrameSource] failed to seed H264 bootstrap from remote answer: {error}"
                );
            }
        }
    }

    fn record_clean_keyframe_anchor(&self, observed_at_ms: f64) {
        self.runtime_stats
            .record_transport_clean_anchor(observed_at_ms, "chain-clean-keyframe-submitted");
    }

    fn should_trigger_thin_stream_stall(&self, now: std::time::Instant) -> bool {
        self.assembling_frame_start.is_some_and(|started_at| {
            now.duration_since(started_at) >= self.assembly_stall_timeout
                && self.current_assembly_packet_count > 0
                && self.current_assembly_packet_count <= self.thin_stream_packet_threshold
        })
    }

    fn should_prioritize_reinject_drain(&self) -> bool {
        self.runtime_stats
            .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
            .flatten()
            .is_some_and(|observation| {
                observation.stage == "queued" && observation.pending_queue_len > 0
            })
    }

    fn reinject_observation_for_ingress(
        &self,
        ingress_kind: RtcVideoIngressKind,
        primary_ssrc: u32,
        sequence_number: u16,
        rtp_timestamp: u32,
        observed_at_ms: f64,
    ) -> Option<XbxEngineVideoRtxReinjectObservation> {
        let repair = match ingress_kind {
            RtcVideoIngressKind::Primary => return None,
            RtcVideoIngressKind::RepairPrimaryPassThrough { repair } => repair,
            RtcVideoIngressKind::RtxReinject { repair } => repair,
        };
        Some(Self::build_reinject_observation(
            repair,
            primary_ssrc,
            sequence_number,
            rtp_timestamp,
            observed_at_ms,
        ))
    }

    fn build_reinject_observation(
        repair: RtcVideoRepairMetadata,
        primary_ssrc: u32,
        sequence_number: u16,
        rtp_timestamp: u32,
        observed_at_ms: f64,
    ) -> XbxEngineVideoRtxReinjectObservation {
        XbxEngineVideoRtxReinjectObservation {
            stage: "adapterRead".to_string(),
            primary_ssrc,
            repair_ssrc: repair.native_ssrc,
            sequence_number,
            rtp_timestamp,
            pending_queue_len: 0,
            native_sequence_number: Some(repair.native_sequence_number),
            matched_head_gap: false,
            matched_nack_range: false,
            matched_pending_gap: false,
            matched_gap_sequence: None,
            matched_nack_first_sequence: None,
            matched_nack_last_sequence: None,
            observed_at_ms,
        }
    }

    fn record_reinject_stage(
        &self,
        observation: &XbxEngineVideoRtxReinjectObservation,
        stage: &str,
        observed_at_ms: f64,
    ) {
        let mut next = observation.clone();
        next.stage = stage.to_string();
        next.observed_at_ms = observed_at_ms;
        self.runtime_stats.record_video_rtx_reinject(next);
    }
}

fn should_trigger_idle_timeout(
    has_received_packet: bool,
    now: std::time::Instant,
    last_packet_time: std::time::Instant,
    idle_timeout: std::time::Duration,
) -> bool {
    // 首包到来前不把“没有媒体包”当成 idle，避免启动/握手期误触发超时诊断。
    has_received_packet && now.duration_since(last_packet_time) > idle_timeout
}

fn should_relax_idle_timeout(
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    feedback_interval_ms: Option<f64>,
) -> bool {
    const SLOW_FEEDBACK_INTERVAL_THRESHOLD_MS: f64 = 350.0;
    matches!(session_target_type, Some(XbxEngineTargetTypeDto::Cloud))
        || feedback_interval_ms.is_some_and(|ms| ms >= SLOW_FEEDBACK_INTERVAL_THRESHOLD_MS)
}

fn resolve_effective_idle_controls(
    base_idle_timeout: std::time::Duration,
    base_idle_hint_cooldown: std::time::Duration,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    feedback_interval_ms: Option<f64>,
) -> (std::time::Duration, std::time::Duration) {
    const ADAPTIVE_IDLE_TIMEOUT_MS: u64 = 700;
    if !should_relax_idle_timeout(session_target_type, feedback_interval_ms) {
        return (base_idle_timeout, base_idle_hint_cooldown);
    }

    // 云侧或慢反馈场景放宽 idle 判定，降低“反馈慢但链路仍在推进”时的误触发。
    let effective_idle_timeout =
        base_idle_timeout.max(std::time::Duration::from_millis(ADAPTIVE_IDLE_TIMEOUT_MS));
    // hint 冷却跟随放宽，避免短时间重复上报 idle。
    let effective_idle_hint_cooldown = base_idle_hint_cooldown.max(effective_idle_timeout);
    (effective_idle_timeout, effective_idle_hint_cooldown)
}

impl RtcVideoFrameSource {
    fn resolve_effective_idle_controls(&self) -> (std::time::Duration, std::time::Duration) {
        let (session_target_type, feedback_interval_ms) = self
            .runtime_stats
            .read(|stats| {
                (
                    stats.session_target_type.clone(),
                    stats
                        .latest_video_twcc_observation
                        .as_ref()
                        .and_then(|observation| observation.feedback_interval_ms),
                )
            })
            .unwrap_or((None, None));
        resolve_effective_idle_controls(
            self.idle_timeout,
            self.idle_hint_cooldown,
            session_target_type.as_ref(),
            feedback_interval_ms,
        )
    }

    pub(super) async fn recv_frame_inner(&mut self) -> Option<AssembledVideoFrame> {
        loop {
            self.maybe_run_nack_maintenance().await;
            if let Some(sample) = self.sample_builder.pop() {
                self.last_packet_time = std::time::Instant::now();
                self.assembling_frame_start = None;
                self.current_assembly_packet_count = 0;
                let payload = sample.data.to_vec();
                self.assembled_frame_count = self.assembled_frame_count.saturating_add(1);
                self.maybe_seed_h264_bootstrap_from_remote_answer();
                let inspection = match self.h264_inspector.inspect_access_unit(&payload) {
                    Ok(inspection) => inspection,
                    Err(error) => {
                        let now_ms = now_ms_f64();
                        crate::xbx_log_error!(
                            "[RtcVideoFrameSource] h264 inspection failed: {error}"
                        );
                        self.set_waiting_for_recovery_keyframe(true);
                        self.timeline_state
                            .on_admission_await_recovery_keyframe(Some("inspectionError"));
                        self.timeline_state.observe_frame(
                            sample.packet_timestamp,
                            now_ms,
                            None,
                            "unknown",
                        );
                        self.timeline_state.mark_frame_closed(
                            sample.packet_timestamp,
                            now_ms,
                            None,
                            "unknown",
                            Some("inspectionError"),
                        );
                        self.record_video_timeline_observation(
                            "frame-inspection-error-await-keyframe",
                            None,
                            Some(sample.packet_timestamp),
                            now_ms,
                        );
                        self.record_anchor_candidate_ledger(
                            Some(sample.packet_timestamp),
                            "frame-inspection-error-await-keyframe",
                            XbxEngineAnchorCandidateState::Rejected,
                            Some(XbxEngineAnchorCandidateFailureReason::Unknown),
                            now_ms,
                        );
                        self.queue_transport_observation(TransportObservation::Admission(
                            TransportAdmissionObservation::AwaitRecoveryKeyframe,
                        ));
                        continue;
                    }
                };
                let inspection_now_ms = now_ms_f64();
                let inspection_admission = resolve_inspection_admission(&inspection);
                let admission_accepted =
                    matches!(inspection_admission, InspectionAdmission::Accept);
                // bootstrap_reject_reason 只描述当前 AU 是否具备自举条件，不代表 delta slice 不能继续承接。
                self.runtime_stats.record_h264_inspection_observation(
                    XbxEngineH264InspectionObservation {
                        observation_id: u64::from(sample.packet_timestamp),
                        frame_rtp_timestamp: Some(sample.packet_timestamp),
                        nal_types: inspection.nal_type_labels(),
                        has_inband_sps: inspection.has_inband_sps,
                        has_inband_pps: inspection.has_inband_pps,
                        committed_sps_present: inspection.committed_sps_present(),
                        committed_pps_present: inspection.committed_pps_present(),
                        slice_headers_valid: inspection.slice_headers_valid,
                        delta_continuation_ready: inspection.delta_continuation_ready(),
                        parameter_sets_changed: inspection.parameter_sets_changed,
                        config_changed: inspection.config_changed,
                        is_idr: inspection.is_idr,
                        bootstrap_ready: inspection.bootstrap_ready,
                        bootstrap_reject_reason: inspection
                            .bootstrap_reject_reason
                            .map(|reason| reason.as_str().to_string()),
                        admission_accepted,
                        observed_at_ms: inspection_now_ms,
                    },
                );
                match inspection_admission {
                    InspectionAdmission::Accept => {}
                    InspectionAdmission::AwaitRecoveryKeyframe => {
                        let now_ms = now_ms_f64();
                        let reject_reason = inspection_bootstrap_reason(&inspection);
                        crate::xbx_log_warn!(
                            "[RtcVideoFrameSource] h264 inspection rejected sample ts={} bootstrap={:?} slice_headers_valid={}",
                            sample.packet_timestamp,
                            inspection.bootstrap_reject_reason,
                            inspection.slice_headers_valid
                        );
                        self.set_waiting_for_recovery_keyframe(true);
                        self.timeline_state
                            .on_admission_await_recovery_keyframe(Some(reject_reason));
                        self.timeline_state.observe_frame(
                            sample.packet_timestamp,
                            now_ms,
                            None,
                            "unknown",
                        );
                        self.timeline_state.mark_frame_closed(
                            sample.packet_timestamp,
                            now_ms,
                            None,
                            "unknown",
                            Some(reject_reason),
                        );
                        self.record_video_timeline_observation(
                            "frame-inspection-rejected-await-keyframe",
                            None,
                            Some(sample.packet_timestamp),
                            now_ms,
                        );
                        let failure_reason = match reject_reason {
                            "bootstrapMissingSps" => {
                                XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingSps
                            }
                            "bootstrapMissingPps" => {
                                XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingPps
                            }
                            "inspectionRejectInvalidSliceHeader" => {
                                XbxEngineAnchorCandidateFailureReason::InspectionRejectedInvalidSliceHeader
                            }
                            _ => XbxEngineAnchorCandidateFailureReason::Unknown,
                        };
                        self.record_anchor_candidate_ledger(
                            Some(sample.packet_timestamp),
                            "frame-inspection-rejected-await-keyframe",
                            XbxEngineAnchorCandidateState::Rejected,
                            Some(failure_reason),
                            now_ms,
                        );
                        self.queue_transport_observation(TransportObservation::Admission(
                            TransportAdmissionObservation::AwaitRecoveryKeyframe,
                        ));
                        continue;
                    }
                }
                let is_keyframe = inspection.is_idr;
                let config_changed = inspection.config_changed;
                let media_dropped_packets = sample
                    .prev_dropped_packets
                    .saturating_sub(sample.prev_padding_packets);
                if media_dropped_packets > 0 {
                    self.sample_loss_burst_count = self.sample_loss_burst_count.saturating_add(1);
                    self.clean_samples_since_loss = 0;
                } else if is_keyframe {
                    self.sample_loss_burst_count = 0;
                    self.clean_samples_since_loss = 0;
                } else if self.sample_loss_burst_count > 0 {
                    self.clean_samples_since_loss = self.clean_samples_since_loss.saturating_add(1);
                    if self.clean_samples_since_loss >= 4 {
                        self.sample_loss_burst_count = 0;
                        self.clean_samples_since_loss = 0;
                    }
                }
                let frame_now_ms = now_ms_f64();
                let frame_importance = if is_keyframe {
                    "keyframe"
                } else if config_changed {
                    "reference"
                } else {
                    "delta"
                };
                let waiting_for_recovery_keyframe = self.waiting_for_recovery_keyframe();
                let allow_soft_reentry_submit = waiting_for_recovery_keyframe
                    && media_dropped_packets == 0
                    && !is_keyframe
                    && self
                        .timeline_state
                        .try_consume_soft_reentry_budget(frame_now_ms, frame_importance);
                let (next_waiting_for_recovery_keyframe, recovery_action) =
                    resolve_recovery_keyframe_action(
                        waiting_for_recovery_keyframe,
                        self.sample_loss_burst_count,
                        media_dropped_packets,
                        is_keyframe,
                        allow_soft_reentry_submit,
                    );
                self.set_waiting_for_recovery_keyframe(next_waiting_for_recovery_keyframe);

                if media_dropped_packets > 0 {
                    self.runtime_stats
                        .add_inbound_video_packet_loss_estimate(media_dropped_packets);
                    crate::xbx_log_warn!(
                        "[RtcVideoFrameSource] media loss detected before sample ts={} dropped_packets={} is_keyframe={}",
                        sample.packet_timestamp,
                        media_dropped_packets,
                        is_keyframe
                    );
                }

                let sample_loss_frame_importance = if is_keyframe {
                    "keyframe"
                } else if self.sample_loss_burst_count >= 2 {
                    "reference"
                } else {
                    "delta"
                };

                match recovery_action {
                    RecoveryKeyframeAction::Submit => {}
                    RecoveryKeyframeAction::DropAndRequestKeyframe => {
                        let nack_started = self
                            .observe_sample_loss_and_nack(
                                sample.packet_timestamp,
                                media_dropped_packets,
                                is_keyframe,
                                sample_loss_frame_importance,
                            )
                            .await;
                        if !nack_started {
                            self.queue_transport_observation(TransportObservation::Loss(
                                TransportLossObservation::PacketLossDetected,
                            ));
                        }
                        continue;
                    }
                    RecoveryKeyframeAction::TriggerWaitKeyframe => {
                        let now_ms = now_ms_f64();
                        self.timeline_state.on_recovery_keyframe_requested();
                        self.record_video_timeline_observation(
                            "chain-recovery-keyframe-requested",
                            None,
                            Some(sample.packet_timestamp),
                            now_ms,
                        );
                        self.queue_transport_observation(TransportObservation::Loss(
                            TransportLossObservation::RecoveryKeyframeRequested,
                        ));
                        continue;
                    }
                    RecoveryKeyframeAction::WaitKeyframe => {
                        let now_ms = now_ms_f64();
                        self.timeline_state
                            .on_admission_await_recovery_keyframe(Some("awaitingRecoveryKeyframe"));
                        self.record_video_timeline_observation(
                            "frame-await-recovery-keyframe",
                            None,
                            Some(sample.packet_timestamp),
                            now_ms,
                        );
                        self.record_anchor_candidate_ledger(
                            Some(sample.packet_timestamp),
                            "frame-await-recovery-keyframe",
                            XbxEngineAnchorCandidateState::AwaitingRecovery,
                            Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
                            now_ms,
                        );
                        self.queue_transport_observation(TransportObservation::Loss(
                            TransportLossObservation::AwaitRecoveryKeyframe,
                        ));
                        continue;
                    }
                }

                if let Some(width) = inspection.width {
                    self.current_width = width;
                }
                if let Some(height) = inspection.height {
                    self.current_height = height;
                }

                let frame_value = FrameValue::new(is_keyframe, config_changed, payload.len());
                self.last_submitted_frame_value = frame_value;
                self.timeline_state.observe_frame(
                    sample.packet_timestamp,
                    frame_now_ms,
                    Some(is_keyframe),
                    frame_importance,
                );
                self.record_video_timeline_observation(
                    "frame-observed",
                    None,
                    Some(sample.packet_timestamp),
                    frame_now_ms,
                );
                if is_keyframe && media_dropped_packets == 0 {
                    self.record_clean_keyframe_anchor(frame_now_ms);
                    self.record_video_timeline_observation(
                        "chain-clean-keyframe-submitted",
                        None,
                        Some(sample.packet_timestamp),
                        frame_now_ms,
                    );
                }
                let assembled_at = std::time::Instant::now();
                self.transport_deadline_tracker
                    .record_frame_arrival(now_ms_f64());
                let (
                    frame_playout_deadline_at_ms,
                    frame_recovery_disposition,
                    frame_unrecoverable_reason,
                    ledger_budget_context,
                ) = self.take_frame_recovery_ledger(sample.packet_timestamp);
                let frame_budget = ledger_budget_context.unwrap_or_else(|| {
                    FrameBudgetContext::for_ingress_materialization_parts(
                        frame_value,
                        frame_playout_deadline_at_ms,
                        frame_unrecoverable_reason.as_deref(),
                    )
                });
                if self.assembled_frame_count == 1 || self.assembled_frame_count.is_power_of_two() {
                    crate::xbx_log_info!(
                        "[RtcVideoFrameSource] assembled frame count={} ts={} len={} keyframe={} bootstrap={}",
                        self.assembled_frame_count,
                        sample.packet_timestamp,
                        payload.len(),
                        is_keyframe,
                        inspection.bootstrap_ready
                    );
                }

                crate::xbx_log_debug!(
                    "[Ingress] NALU Assb OK: size={}B, res={}x{}, is_kf={}, bootstrap={}",
                    payload.len(),
                    self.current_width,
                    self.current_height,
                    is_keyframe,
                    inspection.bootstrap_ready
                );
                let complete_candidate_now_ms = now_ms_f64();
                self.timeline_state.mark_frame_complete_candidate(
                    sample.packet_timestamp,
                    complete_candidate_now_ms,
                    Some(is_keyframe),
                    frame_importance,
                );
                self.record_video_timeline_observation(
                    "frame-complete-candidate",
                    None,
                    Some(sample.packet_timestamp),
                    complete_candidate_now_ms,
                );
                self.record_anchor_candidate_ledger(
                    Some(sample.packet_timestamp),
                    "frame-complete-candidate",
                    XbxEngineAnchorCandidateState::Observed,
                    None,
                    complete_candidate_now_ms,
                );

                if is_keyframe && media_dropped_packets == 0 {
                    self.record_anchor_candidate_ledger(
                        Some(sample.packet_timestamp),
                        "chain-clean-keyframe-submitted",
                        XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
                        None,
                        frame_now_ms,
                    );
                    // 先把当前 clean anchor candidate 写进 timeline，再开软重入窗口，
                    // 避免 arm 时还拿到旧的 anchor candidate。
                    self.timeline_state.on_clean_keyframe_submitted();
                }

                return Some(AssembledVideoFrame {
                    codec: VideoCodec::H264,
                    is_keyframe,
                    config_changed,
                    value: frame_value,
                    budget: frame_budget,
                    width: self.current_width,
                    height: self.current_height,
                    rtp_timestamp: sample.packet_timestamp,
                    frame_playout_deadline_at_ms,
                    frame_recovery_disposition,
                    frame_unrecoverable_reason,
                    assembled_at,
                    h264: inspection,
                    payload: Bytes::from(payload),
                });
            }

            let now = std::time::Instant::now();
            let (effective_idle_timeout, effective_idle_hint_cooldown) =
                self.resolve_effective_idle_controls();
            let idle_timeout = should_trigger_idle_timeout(
                self.received_packet_count > 0,
                now,
                self.last_packet_time,
                effective_idle_timeout,
            );
            let thin_stream_stall = self.should_trigger_thin_stream_stall(now);

            if idle_timeout || thin_stream_stall {
                let timeout_reason = if thin_stream_stall {
                    "streamThinStall"
                } else {
                    "streamIdleTimeout"
                };
                let timeout_source_event = if thin_stream_stall {
                    "timeout-stream-thin-stall"
                } else {
                    "timeout-stream-idle"
                };
                let timeout_now_ms = now_ms_f64();
                self.timeline_state.record_timeout_reason(timeout_reason);
                self.timeline_state.on_timeout_detected();
                self.record_video_timeline_observation(
                    timeout_source_event,
                    None,
                    None,
                    timeout_now_ms,
                );
                self.sample_builder =
                    build_sample_builder(self.max_late_packets, self.jitter_buffer_max_delay);
                self.assembling_frame_start = None;
                self.current_assembly_packet_count = 0;
                self.last_packet_time = now;

                if self.last_idle_hint_time.map_or(true, |t| {
                    now.duration_since(t) >= effective_idle_hint_cooldown
                }) {
                    self.last_idle_hint_time = Some(now);
                    self.queue_transport_observation(if thin_stream_stall {
                        TransportObservation::StreamThinStall
                    } else {
                        TransportObservation::StreamIdleTimeout
                    });
                }
                continue;
            }

            // 当 RTX 已经命中首洞并排进 reinject queue 时，优先给主 reader 一个很短的直接出队窗口。
            // 否则外层固定 50ms timeout 很容易一直打断普通读路径，导致 queued 包迟迟走不到 deliveredPrimary。
            let read_timeout = if self.should_prioritize_reinject_drain() {
                std::time::Duration::from_millis(8)
            } else {
                std::time::Duration::from_millis(50)
            };
            if let Some(observation) = self
                .runtime_stats
                .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
                .flatten()
            {
                if observation.stage == "queued" && observation.pending_queue_len > 0 {
                    self.reinject_read_poll_count = self.reinject_read_poll_count.saturating_add(1);
                    if self.reinject_read_poll_count == 1
                        || self.reinject_read_poll_count.is_power_of_two()
                    {
                        crate::xbx_log_warn!(
                            "[RtcVideoFrameSource] reinjectReadPoll pending={} gap={:?} nack={:?}..{:?} timeout_ms={} count={}",
                            observation.pending_queue_len,
                            observation.matched_gap_sequence,
                            observation.matched_nack_first_sequence,
                            observation.matched_nack_last_sequence,
                            read_timeout.as_millis(),
                            self.reinject_read_poll_count
                        );
                    }
                }
            }
            match tokio::time::timeout(read_timeout, self.rx.recv()).await {
                Ok(Some(rtp_video_packet)) => {
                    self.received_packet_count = self.received_packet_count.saturating_add(1);
                    let ingress_kind = rtp_video_packet.ingress_kind;
                    let rtp = rtp_video_packet.to_rtp_packet();
                    self.last_packet_time = std::time::Instant::now();
                    if self.assembling_frame_start.is_none() {
                        self.assembling_frame_start = Some(self.last_packet_time);
                        self.current_assembly_packet_count = 0;
                    }
                    self.current_assembly_packet_count =
                        self.current_assembly_packet_count.saturating_add(1);
                    let seq = rtp.header.sequence_number;
                    let now_ms = now_ms_f64();
                    let reinject_observation = self.reinject_observation_for_ingress(
                        ingress_kind,
                        rtp.header.ssrc,
                        seq,
                        rtp.header.timestamp,
                        now_ms,
                    );
                    if let Some(observation) = reinject_observation.as_ref() {
                        self.runtime_stats
                            .record_video_rtx_reinject(observation.clone());
                    }
                    let (next_highest_sequence, forward_gap) =
                        detect_forward_gap(self.last_highest_rtp_sequence, seq);
                    self.last_highest_rtp_sequence = next_highest_sequence;
                    if let Some((expected_sequence, received_sequence)) = forward_gap {
                        let missing_sequences = super::nack::wrapping_sequence_range(
                            expected_sequence,
                            received_sequence,
                        );
                        self.timeline_state.observe_gap(
                            &missing_sequences,
                            now_ms,
                            Some(rtp.header.timestamp),
                            "unknown",
                        );
                        if let Some(sequence) = missing_sequences.first().copied() {
                            self.record_video_timeline_observation(
                                "gap-observed-forward-packet",
                                Some(sequence),
                                Some(rtp.header.timestamp),
                                now_ms,
                            );
                        }
                        self.observe_forward_gap_and_nack(expected_sequence, received_sequence)
                            .await;
                    }
                    self.nack_window.add(seq);
                    self.push_recent_rtp_packet(seq, rtp.header.timestamp);
                    if let Some(observation) = reinject_observation.as_ref() {
                        self.record_reinject_stage(observation, "sampleBuilderPush", now_ms);
                    }
                    if let Some(resolved) = self.nack_scheduler.resolve_sequence(seq, now_ms) {
                        self.timeline_state.mark_gap_resolved(
                            seq,
                            now_ms,
                            resolved.frame_rtp_timestamp,
                            resolved.frame_importance,
                        );
                        self.record_video_timeline_observation(
                            "gap-resolved",
                            Some(seq),
                            resolved.frame_rtp_timestamp,
                            now_ms,
                        );
                        self.record_anchor_candidate_ledger(
                            resolved.frame_rtp_timestamp,
                            "gap-resolved",
                            XbxEngineAnchorCandidateState::Repaired,
                            None,
                            now_ms,
                        );
                        if let Some(observation) = reinject_observation.as_ref() {
                            let mut resolved_observation = observation.clone();
                            resolved_observation.matched_nack_range = true;
                            resolved_observation.matched_pending_gap = true;
                            resolved_observation.matched_gap_sequence = Some(seq);
                            resolved_observation.matched_nack_first_sequence = Some(seq);
                            resolved_observation.matched_nack_last_sequence = Some(seq);
                            self.record_reinject_stage(
                                &resolved_observation,
                                "adapterResolved",
                                now_ms,
                            );
                        }
                        self.record_nack_recovered(resolved, now_ms);
                    } else if let Some(observation) = reinject_observation.as_ref() {
                        self.record_reinject_stage(observation, "adapterResolveMiss", now_ms);
                    }
                    if seq % 100 == 0 {
                        crate::xbx_log_info!(
                            "[RtcVideoFrameSource] RTP packet received: seq={}, ts={}",
                            seq,
                            rtp.header.timestamp
                        );
                    }
                    if self.received_packet_count == 1
                        || self.received_packet_count.is_power_of_two()
                    {
                        crate::xbx_log_info!(
                            "[RtcVideoFrameSource] packet received count={} seq={} ts={}",
                            self.received_packet_count,
                            seq,
                            rtp.header.timestamp
                        );
                    }
                    if !matches!(ingress_kind, RtcVideoIngressKind::RtxReinject { .. }) {
                        self.current_media_ssrc = Some(rtp.header.ssrc);
                    }
                    self.sample_builder.push(rtp);
                }
                Ok(None) => {
                    crate::xbx_log_error!("[RtcVideoFrameSource] rx closed");
                    return None;
                }
                Err(_) => {}
            }
        }
    }
}

impl FrameSource for RtcVideoFrameSource {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<AssembledVideoFrame>> + Send + 'a>>
    {
        Box::pin(async move { self.recv_frame_inner().await })
    }
}

impl TransportObservationSource for RtcVideoTransportObservationSource {
    fn recv_transport_observation<'a>(
        &'a mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<TransportObservation>> + Send + 'a>,
    > {
        Box::pin(async move { self.rx.recv().await })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_effective_idle_controls, resolve_inspection_admission,
        resolve_recovery_keyframe_action, should_trigger_idle_timeout, RecoveryKeyframeAction,
        RtcVideoFrameSource,
    };
    use crate::media::video::h264::inspection::H264AccessUnitInspection;
    use crate::media::video::test_fixtures::{
        bootstrap_idr_nalu, bootstrap_pps_nalu, bootstrap_sps_nalu, make_video_source_for_test,
        make_video_rtp_packet, send_bootstrap_access_unit, NoopRtcpPort,
    };
    use crate::transport::rtc::stream::adapter_types::{
        TransportAdmissionObservation, TransportObservation,
    };
    use crate::transport::rtc::stream::packet_types::{
        RtcVideoIngressKind, RtcVideoRepairMetadata,
    };
    use crate::transport::rtc::stream::sink::RtcRtcpSendPort;
    use crate::transport::rtc::stream::video_source::NackSchedulerConfig;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    #[test]
    fn idle_timeout_is_suppressed_before_first_packet() {
        let started_at = Instant::now();
        let later = started_at + Duration::from_millis(500);

        assert!(!should_trigger_idle_timeout(
            false,
            later,
            started_at,
            Duration::from_millis(150),
        ));
        assert!(should_trigger_idle_timeout(
            true,
            later,
            started_at,
            Duration::from_millis(150),
        ));
    }

    #[test]
    fn cloud_profile_relaxes_idle_timeout_and_hint_cooldown() {
        let (idle_timeout, idle_hint_cooldown) = resolve_effective_idle_controls(
            Duration::from_millis(250),
            Duration::from_millis(400),
            Some(&XbxEngineTargetTypeDto::Cloud),
            Some(120.0),
        );

        assert_eq!(idle_timeout, Duration::from_millis(700));
        assert_eq!(idle_hint_cooldown, Duration::from_millis(700));
    }

    #[test]
    fn slow_feedback_relaxes_idle_timeout_even_for_non_cloud() {
        let (idle_timeout, idle_hint_cooldown) = resolve_effective_idle_controls(
            Duration::from_millis(300),
            Duration::from_millis(450),
            Some(&XbxEngineTargetTypeDto::Home),
            Some(500.0),
        );

        assert_eq!(idle_timeout, Duration::from_millis(700));
        assert_eq!(idle_hint_cooldown, Duration::from_millis(700));
    }

    #[test]
    fn clean_anchor_soft_reentry_allows_healthy_delta_to_submit() {
        let (next_waiting_for_recovery_keyframe, recovery_action) =
            resolve_recovery_keyframe_action(true, 0, 0, false, true);

        assert!(!next_waiting_for_recovery_keyframe);
        assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
    }

    #[test]
    fn clean_anchor_soft_reentry_does_not_override_loss_semantics() {
        let (next_waiting_for_recovery_keyframe, recovery_action) =
            resolve_recovery_keyframe_action(true, 0, 1, false, true);

        assert!(!next_waiting_for_recovery_keyframe);
        assert_eq!(
            recovery_action,
            RecoveryKeyframeAction::DropAndRequestKeyframe
        );
    }

    #[test]
    fn recovery_wait_without_soft_reentry_remains_waiting() {
        let (next_waiting_for_recovery_keyframe, recovery_action) =
            resolve_recovery_keyframe_action(true, 0, 0, false, false);

        assert!(next_waiting_for_recovery_keyframe);
        assert_eq!(recovery_action, RecoveryKeyframeAction::WaitKeyframe);
    }

    #[test]
    fn inspection_admission_rejects_frames_without_bootstrap_or_continuation() {
        assert_eq!(
            resolve_inspection_admission(&H264AccessUnitInspection {
                nals: Vec::new(),
                parameter_sets: None,
                width: None,
                height: None,
                is_idr: true,
                has_inband_sps: false,
                has_inband_pps: false,
                slice_headers_valid: true,
                parameter_sets_changed: false,
                config_changed: false,
                bootstrap_ready: true,
                bootstrap_reject_reason: None,
                commit_state:
                    crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
            }),
            super::InspectionAdmission::Accept
        );

        assert_eq!(
            resolve_inspection_admission(&H264AccessUnitInspection {
                nals: Vec::new(),
                parameter_sets: None,
                width: None,
                height: None,
                is_idr: false,
                has_inband_sps: false,
                has_inband_pps: false,
                slice_headers_valid: true,
                parameter_sets_changed: false,
                config_changed: false,
                bootstrap_ready: false,
                bootstrap_reject_reason: None,
                commit_state:
                    crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
            }),
            super::InspectionAdmission::AwaitRecoveryKeyframe
        );

        assert_eq!(
            resolve_inspection_admission(&H264AccessUnitInspection {
                nals: Vec::new(),
                parameter_sets: None,
                width: None,
                height: None,
                is_idr: false,
                has_inband_sps: false,
                has_inband_pps: false,
                slice_headers_valid: false,
                parameter_sets_changed: false,
                config_changed: false,
                bootstrap_ready: false,
                bootstrap_reject_reason: None,
                commit_state:
                    crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
            }),
            super::InspectionAdmission::AwaitRecoveryKeyframe
        );
    }

    #[test]
    fn clean_keyframe_anchor_records_current_transport_recovery_epoch() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(NoopRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        drop(tx);

        source.runtime_stats.begin_transport_recovery_episode(100.0);
        source.record_clean_keyframe_anchor(180.0);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
        assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(180.0));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("chain-clean-keyframe-submitted")
        );
        assert!(!stats.transport_recovery_episode_active);
        assert_eq!(stats.transport_recovery_episode_closed_at_ms, Some(180.0));
        assert_eq!(
            stats.transport_recovery_episode_close_reason.as_deref(),
            Some("cleanAnchor")
        );
    }

    #[tokio::test]
    async fn bootstrap_keyframe_packets_are_assembled_into_frame() {
        let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

        send_bootstrap_access_unit(&tx, 100, 9000).await;
        tx.send(make_video_rtp_packet(103, 9016, true, bootstrap_idr_nalu()))
            .await
            .expect("next frame packet should flush previous sample");
        drop(tx);

        let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("frame assembly should finish")
            .expect("bootstrap frame should be emitted");
        assert!(frame.is_keyframe);
        assert!(frame.h264.bootstrap_ready);
        assert_eq!(frame.rtp_timestamp, 9000);
        assert!(frame.width > 0);
        assert!(frame.height > 0);
        assert!(transport_observation_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn repair_packet_closes_bootstrap_gap_and_allows_frame_assembly() {
        let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

        tx.send(make_video_rtp_packet(100, 9000, false, bootstrap_sps_nalu()))
            .await
            .expect("sps packet should enqueue");
        tx.send(make_video_rtp_packet(102, 9000, true, bootstrap_idr_nalu()))
            .await
            .expect("idr packet should enqueue");
        let mut repair_packet = make_video_rtp_packet(101, 9000, false, bootstrap_pps_nalu());
        repair_packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
            repair: RtcVideoRepairMetadata {
                native_ssrc: 88,
                native_payload_type: 97,
                native_sequence_number: 9_001,
            },
        };
        tx.send(repair_packet)
            .await
            .expect("repair packet should enqueue");
        tx.send(make_video_rtp_packet(103, 9016, true, bootstrap_idr_nalu()))
            .await
            .expect("next frame packet should flush previous sample");
        drop(tx);

        let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("frame assembly should finish")
            .expect("repaired bootstrap frame should be emitted");
        assert!(frame.is_keyframe);
        assert!(frame.h264.bootstrap_ready);
        assert_eq!(frame.rtp_timestamp, 9000);
        assert!(transport_observation_rx.try_recv().is_err());

        let latest = source
            .runtime_stats
            .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
            .flatten()
            .expect("repair observation should be recorded");
        assert_eq!(latest.sequence_number, 101);
        assert_eq!(latest.rtp_timestamp, 9000);
        assert_eq!(latest.native_sequence_number, Some(9_001));
        assert_eq!(latest.repair_ssrc, 88);
    }

    #[tokio::test]
    async fn idr_without_parameter_sets_requests_recovery_keyframe_instead_of_emitting_frame() {
        let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

        tx.send(make_video_rtp_packet(100, 9001, true, bootstrap_idr_nalu()))
            .await
            .expect("idr packet should enqueue");
        tx.send(make_video_rtp_packet(101, 9017, true, bootstrap_idr_nalu()))
            .await
            .expect("follow-up packet should flush previous sample");
        drop(tx);

        let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("reader should finish after rx closes");
        assert!(frame.is_none());

        let observation =
            tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
                .await
                .expect("await-recovery observation should be emitted")
                .expect("observation should exist");
        assert_eq!(
            observation,
            TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
        );
    }

    #[tokio::test]
    async fn bootstrap_packets_without_followup_boundary_do_not_emit_partial_frame() {
        let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

        send_bootstrap_access_unit(&tx, 100, 9000).await;
        drop(tx);

        let frame = tokio::time::timeout(Duration::from_millis(120), source.recv_frame_inner())
            .await
            .expect("reader should finish after rx closes");
        assert!(frame.is_none());
        assert!(transport_observation_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn repair_rtx_packet_keeps_explicit_provenance_through_source_stage_updates() {
        let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();

        let mut packet = make_video_rtp_packet(100, 9_000, true, bootstrap_idr_nalu());
        packet.meta.ssrc = 777;
        packet.meta.payload_type = 124;
        packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
            repair: RtcVideoRepairMetadata {
                native_ssrc: 99,
                native_payload_type: 97,
                native_sequence_number: 4_321,
            },
        };
        tx.send(packet).await.expect("repair packet should enqueue");
        drop(tx);

        let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("reader should finish after rx closes");
        assert!(frame.is_none());

        let latest = source
            .runtime_stats
            .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
            .flatten()
            .expect("repair provenance observation should be recorded");
        assert_eq!(latest.stage, "adapterResolveMiss");
        assert_eq!(latest.sequence_number, 100);
        assert_eq!(latest.repair_ssrc, 99);
        assert_eq!(latest.primary_ssrc, 777);
        assert_eq!(latest.native_sequence_number, Some(4_321));
    }
}
