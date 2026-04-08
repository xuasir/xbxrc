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
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

const FIRST_FRAME_ACQUISITION_MAX_REQUEST_COUNT: u8 = 2;
const SAMPLE_LOSS_WAIT_KEYFRAME_THRESHOLD: u8 = 3;
const SAMPLE_LOSS_BURST_CLEAR_CLEAN_SAMPLE_COUNT: u8 = 6;
const IDLE_TIMEOUT_CONFIRMATION_GRACE_MIN_MS: u64 = 120;
const IDLE_TIMEOUT_CONFIRMATION_GRACE_MAX_MS: u64 = 220;
const THIN_STREAM_CONFIRMATION_GRACE_MIN_MS: u64 = 90;
const THIN_STREAM_CONFIRMATION_GRACE_MAX_MS: u64 = 180;
const WAITING_KEYFRAME_CONTINUATION_WINDOW_MS: f64 = 120.0;
const WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES: u32 = 3;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FirstFrameAcquisitionRequestKind {
    Initial,
    Followup,
}

#[derive(Clone, Copy, Debug, Default)]
struct FirstFrameAcquisitionRuntimeContext {
    session_is_startup: bool,
    transport_connected: bool,
    answer_missing_sprop: bool,
    first_frame_acquired: bool,
    audio_started: bool,
    video_track_audio_only: bool,
    video_track_media_seen: bool,
}

impl FirstFrameAcquisitionRuntimeContext {
    fn chain_active(&self) -> bool {
        self.session_is_startup
            && self.transport_connected
            && self.answer_missing_sprop
            && !self.first_frame_acquired
    }

    fn followup_evidence_ready(&self) -> bool {
        self.audio_started || self.video_track_audio_only || self.video_track_media_seen
    }
}

pub(super) fn resolve_inspection_admission(
    inspection: &H264AccessUnitInspection,
    first_frame_acquired: bool,
    decoder_bootstrap_no_output_continuation_allowed: bool,
) -> InspectionAdmission {
    if !inspection.slice_headers_valid {
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    if inspection.bootstrap_ready {
        return InspectionAdmission::Accept;
    }

    if (first_frame_acquired || decoder_bootstrap_no_output_continuation_allowed)
        && inspection.delta_continuation_ready()
    {
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

fn keyframe_episode_response_detail(
    inspection: &H264AccessUnitInspection,
    admission: InspectionAdmission,
) -> &'static str {
    if !inspection.is_idr {
        return "firstResponseNonKeyframe";
    }
    match admission {
        InspectionAdmission::Accept => "firstKeyframeAccepted",
        InspectionAdmission::AwaitRecoveryKeyframe => inspection_bootstrap_reason(inspection),
    }
}

pub(super) fn resolve_recovery_keyframe_action(
    first_frame_acquired: bool,
    waiting_for_recovery_keyframe: bool,
    hard_recovery_gap_risk: bool,
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
        // 小抖动先留在 drop + request keyframe，只有连续坏窗才进入 waitKeyframe。
        if sample_loss_burst_count >= SAMPLE_LOSS_WAIT_KEYFRAME_THRESHOLD {
            return (true, RecoveryKeyframeAction::TriggerWaitKeyframe);
        }
        return (false, RecoveryKeyframeAction::DropAndRequestKeyframe);
    }

    if waiting_for_recovery_keyframe {
        if !first_frame_acquired {
            return (true, RecoveryKeyframeAction::WaitKeyframe);
        }
        if allow_soft_reentry_submit {
            // clean anchor 后的短窗内，健康 delta 只要还能安全提交，就别继续把链路拖回 recovering。
            return (false, RecoveryKeyframeAction::Submit);
        }
        if !hard_recovery_gap_risk {
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
    fn decoder_bootstrap_no_output_continuation_allowed(
        &self,
        inspection: &H264AccessUnitInspection,
        first_frame_acquired: bool,
        now_ms: f64,
    ) -> bool {
        if first_frame_acquired
            || inspection.is_idr
            || inspection.bootstrap_ready
            || !inspection.slice_headers_valid
            || !inspection.delta_continuation_ready()
            || !inspection.committed_sps_present()
            || !inspection.committed_pps_present()
            || !self.waiting_for_recovery_keyframe()
        {
            return false;
        }

        self.runtime_stats
            .read(|stats| {
                let Some(observation) = stats.latest_decode_output_path_observation.as_ref() else {
                    return false;
                };
                if observation.verdict != "backend-no-output"
                    || observation.detail != "backendNoOutputAfterBootstrapKeyframe"
                    || !observation.is_keyframe
                {
                    return false;
                }
                let age_ms = now_ms - observation.observed_at_ms;
                if !(0.0..=WAITING_KEYFRAME_CONTINUATION_WINDOW_MS).contains(&age_ms) {
                    return false;
                }
                observation
                    .input_frames_since_last_decoded
                    .is_some_and(|count| {
                        count <= WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES.saturating_add(1)
                    })
            })
            .unwrap_or(false)
    }

    fn clear_pending_timeout_confirmations(&mut self) {
        self.pending_idle_timeout_since = None;
        self.pending_thin_stream_since = None;
    }

    fn first_frame_acquired(stats: &crate::XbxEngineMediaRuntimeStats) -> bool {
        stats.latest_video_decode_ok_time_ms.is_some()
            || stats.latest_video_host_present_time_ms.is_some()
            || stats.video_present_submit_count_total > 0
            || stats.video_present_epoch > 0
    }

    fn first_frame_acquisition_runtime_context(
        &self,
    ) -> Option<FirstFrameAcquisitionRuntimeContext> {
        self.runtime_stats.read(|stats| {
            let video_track = stats.latest_video_track_status.as_ref();
            FirstFrameAcquisitionRuntimeContext {
                session_is_startup: matches!(
                    stats.session_phase.as_deref(),
                    Some("startup" | "handshaking" | "priming")
                ),
                transport_connected: stats.transport_state == XbxEngineTransportStateDto::Connected,
                answer_missing_sprop: stats.latest_remote_answer_observation.as_ref().is_some_and(
                    |observation| {
                        observation.selected_video_mime_type.as_deref() == Some("video/h264")
                            && observation
                                .selected_video_h264_sprop_parameter_sets
                                .as_ref()
                                .is_none_or(|sets| sets.is_empty())
                    },
                ),
                first_frame_acquired: Self::first_frame_acquired(stats),
                audio_started: stats.latest_audio_playout_time_ms.is_some(),
                video_track_audio_only: video_track
                    .is_some_and(|track| track.state.as_str() == "audioOnly"),
                video_track_media_seen: video_track.is_some_and(|track| {
                    track.video_bytes_total > 0 || track.video_packet_count_total > 0
                }),
            }
        })
    }

    fn should_request_first_frame_acquisition_keyframe(
        &self,
        request_kind: FirstFrameAcquisitionRequestKind,
    ) -> bool {
        if self.first_frame_acquisition_keyframe_request_count
            >= FIRST_FRAME_ACQUISITION_MAX_REQUEST_COUNT
        {
            return false;
        }

        let Some(context) = self.first_frame_acquisition_runtime_context() else {
            return false;
        };
        if !context.chain_active() {
            return false;
        }

        match request_kind {
            FirstFrameAcquisitionRequestKind::Initial => {
                self.first_frame_acquisition_keyframe_request_count == 0
            }
            FirstFrameAcquisitionRequestKind::Followup => {
                self.first_frame_acquisition_keyframe_request_count == 1
                    && context.followup_evidence_ready()
            }
        }
    }

    fn should_request_first_frame_acquisition_followup_keyframe(
        &self,
        inspection: &H264AccessUnitInspection,
    ) -> bool {
        let reject_reason_matches = matches!(
            inspection.bootstrap_reject_reason,
            Some(
                H264BootstrapRejectReason::MissingSps
                    | H264BootstrapRejectReason::MissingPps
                    | H264BootstrapRejectReason::NonIdrVcl
                    | H264BootstrapRejectReason::InvalidSliceHeader
            )
        ) || !inspection.slice_headers_valid;
        reject_reason_matches
            && !inspection.bootstrap_ready
            && self.should_request_first_frame_acquisition_keyframe(
                FirstFrameAcquisitionRequestKind::Followup,
            )
    }

    fn maybe_request_first_frame_acquisition_keyframe(
        &mut self,
        frame_rtp_timestamp: Option<u32>,
        request_kind: FirstFrameAcquisitionRequestKind,
    ) {
        if !self.should_request_first_frame_acquisition_keyframe(request_kind) {
            return;
        }
        self.first_frame_acquisition_keyframe_request_count = self
            .first_frame_acquisition_keyframe_request_count
            .saturating_add(1);
        let now_ms = now_ms_f64();
        self.timeline_state.on_recovery_keyframe_requested();
        let event_name = match request_kind {
            FirstFrameAcquisitionRequestKind::Initial => {
                "first-frame-acquisition-keyframe-requested"
            }
            FirstFrameAcquisitionRequestKind::Followup => {
                "first-frame-acquisition-keyframe-followup-requested"
            }
        };
        self.record_video_timeline_observation(event_name, None, frame_rtp_timestamp, now_ms);
        self.queue_transport_observation(TransportObservation::Loss(
            TransportLossObservation::RecoveryKeyframeRequested,
        ));
    }

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

    fn thin_stream_confirmation_grace(&self) -> std::time::Duration {
        self.assembly_stall_timeout.div_f32(3.0).clamp(
            std::time::Duration::from_millis(THIN_STREAM_CONFIRMATION_GRACE_MIN_MS),
            std::time::Duration::from_millis(THIN_STREAM_CONFIRMATION_GRACE_MAX_MS),
        )
    }

    fn idle_timeout_confirmation_grace(
        &self,
        idle_timeout: std::time::Duration,
    ) -> std::time::Duration {
        idle_timeout.div_f32(2.0).clamp(
            std::time::Duration::from_millis(IDLE_TIMEOUT_CONFIRMATION_GRACE_MIN_MS),
            std::time::Duration::from_millis(IDLE_TIMEOUT_CONFIRMATION_GRACE_MAX_MS),
        )
    }

    fn should_confirm_transient_timeout_signal(
        pending_since: &mut Option<std::time::Instant>,
        now: std::time::Instant,
        confirmation_grace: std::time::Duration,
    ) -> bool {
        match pending_since {
            Some(first_seen_at) if now.duration_since(*first_seen_at) >= confirmation_grace => {
                *pending_since = None;
                true
            }
            Some(_) => false,
            None => {
                *pending_since = Some(now);
                false
            }
        }
    }

    fn should_emit_confirmed_idle_timeout(
        &mut self,
        now: std::time::Instant,
        idle_timeout: std::time::Duration,
    ) -> bool {
        let confirmation_grace = self.idle_timeout_confirmation_grace(idle_timeout);
        Self::should_confirm_transient_timeout_signal(
            &mut self.pending_idle_timeout_since,
            now,
            confirmation_grace,
        )
    }

    fn should_emit_confirmed_thin_stream_stall(&mut self, now: std::time::Instant) -> bool {
        let confirmation_grace = self.thin_stream_confirmation_grace();
        Self::should_confirm_transient_timeout_signal(
            &mut self.pending_thin_stream_since,
            now,
            confirmation_grace,
        )
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

fn idle_timeout_render_slack_window_ms(idle_timeout: std::time::Duration) -> f64 {
    const IDLE_TIMEOUT_SLACK_WINDOW_MIN_MS: f64 = 220.0;
    const IDLE_TIMEOUT_SLACK_WINDOW_MAX_MS: f64 = 450.0;
    let scaled = (idle_timeout.as_millis() as f64) * 1.5;
    scaled
        .max(IDLE_TIMEOUT_SLACK_WINDOW_MIN_MS)
        .min(IDLE_TIMEOUT_SLACK_WINDOW_MAX_MS)
}

fn should_absorb_idle_timeout_for_steady_gap(
    transport_state: XbxEngineTransportStateDto,
    current_recovery_epoch: u64,
    clean_anchor_epoch: Option<u64>,
    clean_anchor_source_event: Option<&str>,
    latest_video_host_present_time_ms: Option<f64>,
    latest_video_decode_ok_time_ms: Option<f64>,
    video_renderer_stalled: Option<bool>,
    video_decoder_stalled: Option<bool>,
    now_ms: f64,
    idle_timeout: std::time::Duration,
) -> bool {
    if transport_state != XbxEngineTransportStateDto::Connected {
        return false;
    }
    if video_renderer_stalled.unwrap_or(false) || video_decoder_stalled.unwrap_or(false) {
        return false;
    }
    let has_current_clean_anchor = clean_anchor_epoch.is_some_and(|epoch| {
        epoch == current_recovery_epoch
            && clean_anchor_source_event == Some("chain-clean-keyframe-submitted")
    });
    if !has_current_clean_anchor {
        return false;
    }
    let fresh_window_ms = idle_timeout_render_slack_window_ms(idle_timeout);
    let present_fresh = latest_video_host_present_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= fresh_window_ms);
    let decode_fresh = latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= fresh_window_ms);
    present_fresh || decode_fresh
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

    fn should_absorb_idle_timeout(&self, idle_timeout: std::time::Duration) -> bool {
        if self.timeline_state.waiting_for_recovery_keyframe() {
            return false;
        }
        let now_ms = now_ms_f64();
        self.runtime_stats
            .read(|stats| {
                should_absorb_idle_timeout_for_steady_gap(
                    stats.transport_state.clone(),
                    stats.transport_recovery_epoch,
                    stats.video_anchor_clean_epoch,
                    stats.video_anchor_clean_source_event.as_deref(),
                    stats.latest_video_host_present_time_ms,
                    stats.latest_video_decode_ok_time_ms,
                    stats.video_renderer_stalled,
                    stats.video_decoder_stalled,
                    now_ms,
                    idle_timeout,
                )
            })
            .unwrap_or(false)
    }

    pub(super) async fn recv_frame_inner(&mut self) -> Option<AssembledVideoFrame> {
        loop {
            self.maybe_run_nack_maintenance().await;
            if let Some(sample) = self.sample_builder.pop() {
                self.clear_pending_timeout_confirmations();
                self.last_packet_time = std::time::Instant::now();
                self.assembling_frame_start = None;
                self.current_assembly_packet_count = 0;
                let payload = sample.data.to_vec();
                self.assembled_frame_count = self.assembled_frame_count.saturating_add(1);
                self.maybe_request_first_frame_acquisition_keyframe(
                    Some(sample.packet_timestamp),
                    FirstFrameAcquisitionRequestKind::Initial,
                );
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
                let first_frame_acquired = self
                    .runtime_stats
                    .read(|stats| Self::first_frame_acquired(stats))
                    .unwrap_or(false);
                let decoder_bootstrap_no_output_continuation_allowed = self
                    .decoder_bootstrap_no_output_continuation_allowed(
                        &inspection,
                        first_frame_acquired,
                        inspection_now_ms,
                    );
                let inspection_admission = resolve_inspection_admission(
                    &inspection,
                    first_frame_acquired,
                    decoder_bootstrap_no_output_continuation_allowed,
                );
                let admission_accepted =
                    matches!(inspection_admission, InspectionAdmission::Accept);
                // bootstrap_reject_reason 只描述当前 AU 是否具备自举条件，不代表 delta slice 不能继续承接。
                self.runtime_stats.record_h264_inspection_observation(
                    XbxEngineH264InspectionObservation {
                        observation_id: u64::from(sample.packet_timestamp),
                        frame_rtp_timestamp: Some(sample.packet_timestamp),
                        nal_types: inspection.nal_type_labels(),
                        nal_count: inspection.nals.len() as u16,
                        vcl_nal_count: inspection
                            .nals
                            .iter()
                            .filter(|nal| matches!(
                                nal.unit_type,
                                h264_reader::nal::UnitType::SliceLayerWithoutPartitioningIdr
                                    | h264_reader::nal::UnitType::SliceLayerWithoutPartitioningNonIdr
                            ))
                            .count() as u16,
                        has_inband_sps: inspection.has_inband_sps,
                        has_inband_pps: inspection.has_inband_pps,
                        committed_sps_present: inspection.committed_sps_present(),
                        committed_pps_present: inspection.committed_pps_present(),
                        slice_headers_valid: inspection.slice_headers_valid,
                        delta_continuation_ready: inspection.delta_continuation_ready(),
                        parameter_sets_changed: inspection.parameter_sets_changed,
                        config_changed: inspection.config_changed,
                        is_idr: inspection.is_idr,
                        sample_width: inspection.width,
                        sample_height: inspection.height,
                        bootstrap_ready: inspection.bootstrap_ready,
                        bootstrap_reject_reason: inspection
                            .bootstrap_reject_reason
                            .map(|reason| reason.as_str().to_string()),
                        admission_accepted,
                        observed_at_ms: inspection_now_ms,
                    },
                );
                self.runtime_stats
                    .record_keyframe_request_episode_response_observed(
                        inspection_now_ms,
                        Some(sample.packet_timestamp),
                        inspection.is_idr,
                        keyframe_episode_response_detail(&inspection, inspection_admission),
                    );
                if self.should_request_first_frame_acquisition_followup_keyframe(&inspection) {
                    self.maybe_request_first_frame_acquisition_keyframe(
                        Some(sample.packet_timestamp),
                        FirstFrameAcquisitionRequestKind::Followup,
                    );
                }
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
                    if self.clean_samples_since_loss >= SAMPLE_LOSS_BURST_CLEAR_CLEAN_SAMPLE_COUNT {
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
                let hard_recovery_gap_risk = self.timeline_state.has_hard_recovery_gap_risk();
                let allow_soft_reentry_submit = waiting_for_recovery_keyframe
                    && first_frame_acquired
                    && media_dropped_packets == 0
                    && !is_keyframe
                    && self
                        .timeline_state
                        .try_consume_soft_reentry_budget(frame_now_ms, frame_importance);
                let (next_waiting_for_recovery_keyframe, recovery_action) =
                    resolve_recovery_keyframe_action(
                        first_frame_acquired,
                        waiting_for_recovery_keyframe,
                        hard_recovery_gap_risk,
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
                } else if self.sample_loss_burst_count >= SAMPLE_LOSS_WAIT_KEYFRAME_THRESHOLD {
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
            let idle_timeout =
                idle_timeout && !self.should_absorb_idle_timeout(effective_idle_timeout);
            let thin_stream_stall = self.should_trigger_thin_stream_stall(now);
            let idle_timeout = if idle_timeout {
                self.should_emit_confirmed_idle_timeout(now, effective_idle_timeout)
            } else {
                self.pending_idle_timeout_since = None;
                false
            };
            let thin_stream_stall = if thin_stream_stall {
                self.should_emit_confirmed_thin_stream_stall(now)
            } else {
                self.pending_thin_stream_since = None;
                false
            };

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
                    self.clear_pending_timeout_confirmations();
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
#[path = "source.test.rs"]
mod tests;
