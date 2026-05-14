use super::nack_policy::RECOVERY_KEYFRAME_RETRY_MAX_COUNT;
use super::nack_window::{NackWindowAddOutcome, SequenceRange};
use super::{
    build_sample_builder, nack::gap_transport_evidence, now_ms_f64, timeline::ChainState,
    UINT16SIZE_HALF,
};
use crate::media::video::h264::inspection::{
    H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason,
};
use base64::Engine as _;
use bytes::Bytes;
use rtc_rtp::packet::Packet;

use crate::media::video::ingress::budget::{
    FrameBudgetContext, FrameBudgetRttSlack, FrameBudgetWindowSource,
};
use crate::media::video::types::{
    AssembledVideoFrame, FrameRecoveryDisposition, FrameValue, VideoCodec,
};
use crate::transport::rtc::recovery::contract::{
    is_recovery_delta_continuation_ready, FrameValue as RecoveryFrameValue,
};
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
const SAMPLE_LOSS_BURST_CLEAR_CLEAN_SAMPLE_COUNT: u8 = 6;
const IDLE_TIMEOUT_CONFIRMATION_GRACE_MIN_MS: u64 = 120;
const IDLE_TIMEOUT_CONFIRMATION_GRACE_MAX_MS: u64 = 220;
const THIN_STREAM_CONFIRMATION_GRACE_MIN_MS: u64 = 90;
const THIN_STREAM_CONFIRMATION_GRACE_MAX_MS: u64 = 180;
const WAITING_KEYFRAME_CONTINUATION_WINDOW_MS: f64 = 120.0;
const WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES: u32 = 3;
const OOS_ACTIVITY_COOLDOWN_MS: f64 = 30_000.0;
const OOS_DEPTH_WINDOW_CAPACITY: usize = 64;
const OOS_SKIP_LAST_N_REFRESH_INTERVAL_MS: f64 = 200.0;
const FRAME_OOS_TRACK_CAPACITY: usize = 64;
const HEAD_MISSING_ACTIVITY_COOLDOWN_MS: f64 = 30_000.0;
const FRAME_PLAYOUT_BASE_TRACK_CAPACITY: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecoveryKeyframeAction {
    Submit,
    DropAndRequestPli,
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
    prior_output_continuation_allowed: bool,
    decoder_bootstrap_no_output_continuation_allowed: bool,
    sustaining_recovery_continuation_allowed: bool,
) -> InspectionAdmission {
    if !inspection.slice_headers_valid {
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    if inspection.bootstrap_ready {
        return InspectionAdmission::Accept;
    }

    if (prior_output_continuation_allowed
        || decoder_bootstrap_no_output_continuation_allowed
        || sustaining_recovery_continuation_allowed)
        && is_recovery_delta_continuation_ready(inspection)
    {
        return InspectionAdmission::Accept;
    }

    InspectionAdmission::AwaitRecoveryKeyframe
}

fn prior_output_continuation_allowed(
    first_frame_acquired: bool,
    is_blocking_non_keyframe_admission: bool,
    chain_requires_recovery_anchor: bool,
    clean_anchor_building_phase_active: bool,
) -> bool {
    first_frame_acquired
        && (clean_anchor_building_phase_active
            || (!is_blocking_non_keyframe_admission && !chain_requires_recovery_anchor))
}

/// RFC 2026-05-14：在 IDR AU 缺 in-band SPS/PPS 但 decoder 侧已有稳定 committed 参数集时，尝试 prepend 后重检。
fn try_h264_bootstrap_ps_salvage_au(
    inspector: &H264AccessUnitInspector,
    inspection: &H264AccessUnitInspection,
    payload: &[u8],
) -> Option<Vec<u8>> {
    if inspection.bootstrap_ready {
        return None;
    }
    let reject = inspection.bootstrap_reject_reason.as_ref()?;
    if !matches!(
        reject,
        H264BootstrapRejectReason::MissingSps | H264BootstrapRejectReason::MissingPps
    ) {
        return None;
    }
    if !inspection.is_idr {
        return None;
    }
    if inspection.parameter_sets_changed || inspection.config_changed {
        return None;
    }
    if !inspection.slice_headers_valid {
        return None;
    }
    if !inspection.committed_sps_present() || !inspection.committed_pps_present() {
        return None;
    }
    let mut prefix = inspector.committed_parameter_set_annex_b_prefix()?;
    let mut out = Vec::with_capacity(prefix.len() + payload.len());
    out.append(&mut prefix);
    out.extend_from_slice(payload);
    Some(out)
}

fn inspection_bootstrap_reason(inspection: &H264AccessUnitInspection) -> &'static str {
    match inspection.bootstrap_reject_reason {
        Some(H264BootstrapRejectReason::NoVcl) => "inspectionRejectNoVcl",
        Some(H264BootstrapRejectReason::MissingSps) => "bootstrapMissingSps",
        Some(H264BootstrapRejectReason::MissingPps) => "bootstrapMissingPps",
        Some(H264BootstrapRejectReason::BootstrapMissingIdr)
        | Some(H264BootstrapRejectReason::NonIdrVcl) => "bootstrapMissingIdr",
        Some(H264BootstrapRejectReason::MixedIdrWithTrailingDelta) => "mixedIdrWithTrailingDelta",
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
        if matches!(admission, InspectionAdmission::Accept)
            && inspection.delta_continuation_ready()
            && inspection.committed_sps_present()
            && inspection.committed_pps_present()
        {
            return "continuationAcceptedWhileAwaitingIdr";
        }
        return "bootstrapMissingIdr";
    }
    match admission {
        InspectionAdmission::Accept => "firstKeyframeAccepted",
        InspectionAdmission::AwaitRecoveryKeyframe => inspection_bootstrap_reason(inspection),
    }
}

pub(super) fn resolve_recovery_keyframe_action(
    first_frame_acquired: bool,
    is_blocking_non_keyframe_admission: bool,
    sustaining_recovery_active: bool,
    hard_recovery_gap_risk: bool,
    clean_anchor_building_phase_active: bool,
    _sample_loss_burst_count: u8,
    media_dropped_packets: u16,
    is_keyframe: bool,
) -> (bool, RecoveryKeyframeAction) {
    // 带丢包的 keyframe/reference 不能继续喂给解码器，否则很容易把本地参考链喂脏，
    // 在 macOS 上会直接放大成 VideoToolbox 连续 bad-data 回调。
    if is_keyframe && media_dropped_packets > 0 {
        // 这里仍只保留 decoder safety：丢弃坏 keyframe，但恢复升级交给统一 NACK/recovery admission。
        return (false, RecoveryKeyframeAction::DropAndRequestPli);
    }

    if is_keyframe {
        return (false, RecoveryKeyframeAction::Submit);
    }

    if media_dropped_packets > 0 {
        // sample loss 的升级门交给统一 NACK/recovery admission；source 这里只保留解码安全职责。
        return (false, RecoveryKeyframeAction::DropAndRequestPli);
    }

    if is_blocking_non_keyframe_admission {
        if !first_frame_acquired {
            return (true, RecoveryKeyframeAction::WaitKeyframe);
        }
        if sustaining_recovery_active {
            // 正式恢复保活阶段的目标是先维持连续输出，而不是再次掉回 wait-keyframe。
            // 只要当前帧仍是健康 continuation，就继续提交，真正的稳定退出交给 timeline gate。
            return (false, RecoveryKeyframeAction::Submit);
        }
        if clean_anchor_building_phase_active {
            // clean-anchor 已经提交并处在建链窗口时，允许 continuation 把链路重新带活；
            // 真正的稳定退出仍由 clean-anchor / display gate 统一裁决。
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
    fn h264_continuation_verdict(
        inspection: &H264AccessUnitInspection,
        admission: InspectionAdmission,
    ) -> Option<String> {
        let continuation_ready = inspection.delta_continuation_ready()
            && inspection.committed_sps_present()
            && inspection.committed_pps_present();
        if matches!(admission, InspectionAdmission::Accept)
            && !inspection.bootstrap_ready
            && continuation_ready
        {
            return Some("continuationAcceptedWhileAwaitingIdr".to_string());
        }
        if continuation_ready {
            return Some("continuationReady".to_string());
        }
        None
    }

    fn maybe_ack_clean_anchor_commit_from_runtime_stats(&mut self) {
        let committed_submission = self.runtime_stats.read(|stats| {
            let committed_epoch = stats.latest_clean_anchor_submission_epoch;
            let committed_rtp_timestamp = stats.latest_clean_anchor_submission_rtp_timestamp;
            let clean_anchor_source_event =
                stats.latest_clean_anchor_submission_source_event.as_deref();
            (
                committed_epoch,
                committed_rtp_timestamp,
                clean_anchor_source_event == Some("chain-clean-anchor-submitted"),
            )
        });
        let Some((Some(committed_epoch), committed_rtp_timestamp, committed_submission)) =
            committed_submission
        else {
            return;
        };
        if !committed_submission || committed_epoch == self.last_consumed_clean_anchor_epoch {
            return;
        }
        let acked = if let Some(committed_rtp_timestamp) = committed_rtp_timestamp {
            self.timeline_state
                .ack_clean_anchor_stats_committed(committed_rtp_timestamp)
        } else {
            self.timeline_state
                .ack_pending_clean_anchor_stats_committed()
        };
        if acked {
            self.last_consumed_clean_anchor_epoch = committed_epoch;
        }
    }

    async fn handle_drop_and_request_keyframe_action(
        &mut self,
        sample_rtp_timestamp: u32,
        media_dropped_packets: u16,
        is_keyframe: bool,
        media_type_label: &'static str,
    ) {
        let nack_started = self
            .observe_sample_loss_and_nack(
                sample_rtp_timestamp,
                media_dropped_packets,
                is_keyframe,
                media_type_label,
            )
            .await;
        if nack_started {
            return;
        }

        // NACK 无法定位有效缺失序列号时，直接升级为显式 keyframe 恢复请求，
        // 避免仅上报 PacketLossDetected 后链路没有后续恢复动作。
        let now_ms = now_ms_f64();
        let should_soft_request = self
            .runtime_stats
            .read(|stats| {
                Self::should_soft_request_recovery_keyframe(stats, now_ms, None, false, true, false)
            })
            .unwrap_or(false);
        if should_soft_request {
            self.request_recovery_keyframe_soft_from_source(
                "frame-drop-loss-no-missing-seq-soft-request",
                Some(sample_rtp_timestamp),
                now_ms,
            );
        } else {
            self.set_is_blocking_non_keyframe_admission(true);
            self.timeline_state
                .on_admission_await_recovery_keyframe(Some("sampleLossNoMissingSequence"));
            self.record_video_timeline_observation(
                "frame-drop-loss-no-missing-seq-await-recovery",
                None,
                Some(sample_rtp_timestamp),
                now_ms,
            );
            self.queue_transport_observation(TransportObservation::Admission(
                TransportAdmissionObservation::AwaitRecoveryKeyframe,
            ));
            self.request_recovery_keyframe_from_source(
                "frame-drop-loss-no-missing-seq-hard-request",
                Some(sample_rtp_timestamp),
                now_ms,
            );
        }
        self.record_anchor_candidate_ledger(
            Some(sample_rtp_timestamp),
            "frame-drop-loss-no-missing-seq",
            XbxEngineAnchorCandidateState::Rejected,
            Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
            now_ms,
        );
        self.queue_transport_observation(TransportObservation::Loss(
            TransportLossObservation::PacketLossDetected,
        ));
    }

    fn decoder_feedback_allows_sustaining_exit(&self, now_ms: f64) -> bool {
        const SUSTAINING_EXIT_DECODE_OK_MAX_AGE_MS: f64 = 300.0;
        self.runtime_stats
            .read(|stats| {
                if stats.video_decoder_stalled == Some(true)
                    || stats.video_renderer_stalled == Some(true)
                {
                    return false;
                }
                stats.latest_video_decode_ok_time_ms.is_some_and(|at_ms| {
                    (now_ms - at_ms).max(0.0) <= SUSTAINING_EXIT_DECODE_OK_MAX_AGE_MS
                })
            })
            .unwrap_or(true)
    }

    fn maybe_emit_jitter_early_boundary(&mut self) {
        if !self.jitter_early_emit_enabled {
            return;
        }
        let Some(boundary) = self.pending_marker_boundary else {
            return;
        };
        if boundary.observed_at.elapsed() < self.jitter_early_emit_wait {
            return;
        }
        // 合成边界包：payload 必须至少 2 字节，否则 H264Packet::is_partition_head 返回 false，
        // depacketize 也会因 ErrShortPacket 失败。用 AUD NAL（type=9）作为最小合法 payload，
        // 它不携带图像数据，不会污染帧内容，且 SampleBuilder 会把它当作独立 partition head。
        // timestamp 偏移 +1 确保 SampleBuilder 识别为新帧边界，从而触发上一帧的 pop()。
        let synthetic = Packet {
            header: rtc_rtp::header::Header {
                version: 2,
                marker: false,
                payload_type: boundary.media_payload_type,
                sequence_number: boundary.sequence.wrapping_add(1),
                timestamp: boundary.rtp_timestamp.wrapping_add(1),
                ssrc: self.current_media_ssrc.unwrap_or_default(),
                ..Default::default()
            },
            // AUD NAL unit (type=9): 0x09 = forbidden_zero_bit(0) | nal_ref_idc(00) | nal_unit_type(01001)
            payload: Bytes::from_static(&[0x09, 0xF0]),
        };
        self.sample_builder.push(synthetic);
        self.pending_marker_boundary = None;
        self.jitter_early_emit_count = self.jitter_early_emit_count.saturating_add(1);
        if self.jitter_early_emit_count == 1 || self.jitter_early_emit_count.is_power_of_two() {
            crate::xbx_log_info!(
                "[RtcVideoFrameSource] jitter early emit injected count={} ts={} wait_ms={}",
                self.jitter_early_emit_count,
                boundary.rtp_timestamp,
                boundary.observed_at.elapsed().as_millis()
            );
        }
    }

    fn mark_frame_head_missing_signal(&mut self, rtp_timestamp: u32) {
        if let Some((_, flag)) = self
            .frame_head_missing_flags
            .iter_mut()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
        {
            *flag = true;
        } else {
            if self.frame_head_missing_flags.len() >= FRAME_OOS_TRACK_CAPACITY {
                self.frame_head_missing_flags.pop_front();
            }
            self.frame_head_missing_flags
                .push_back((rtp_timestamp, true));
        }
        self.recent_head_missing_active_until_ms =
            Some(now_ms_f64() + HEAD_MISSING_ACTIVITY_COOLDOWN_MS);
        self.jitter_head_missing_signal_count =
            self.jitter_head_missing_signal_count.saturating_add(1);
        if self.jitter_head_missing_signal_count == 1
            || self.jitter_head_missing_signal_count.is_power_of_two()
        {
            crate::xbx_log_warn!(
                "[RtcVideoFrameSource] frame head-missing signal count={} ts={}",
                self.jitter_head_missing_signal_count,
                rtp_timestamp
            );
        }
    }

    pub(super) fn frame_seen_head_missing(&self, rtp_timestamp: u32) -> bool {
        self.frame_head_missing_flags
            .iter()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
            .is_some_and(|(_, flag)| *flag)
    }

    pub(super) fn head_missing_recently_active(&self, now_ms: f64) -> bool {
        self.recent_head_missing_active_until_ms
            .is_some_and(|until_ms| now_ms <= until_ms)
    }

    pub(super) fn record_frame_drop_attribution(&mut self, rtp_timestamp: u32, dropped: u16) {
        if dropped == 0 {
            return;
        }
        if let Some((_, count)) = self
            .frame_drop_buckets
            .iter_mut()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
        {
            *count = count.saturating_add(dropped);
            return;
        }
        if self.frame_drop_buckets.len() >= FRAME_OOS_TRACK_CAPACITY {
            self.frame_drop_buckets.pop_front();
        }
        self.frame_drop_buckets.push_back((rtp_timestamp, dropped));
    }

    pub(super) fn attributed_drop_count_for_frame(&self, rtp_timestamp: u32, fallback: u16) -> u16 {
        self.frame_drop_buckets
            .iter()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
            .map(|(_, count)| *count)
            .unwrap_or(fallback)
    }

    fn remember_frame_playout_base_candidate(
        &mut self,
        rtp_timestamp: u32,
        observed_at: std::time::Instant,
    ) {
        if self
            .frame_playout_base_times
            .iter()
            .any(|(timestamp, _)| *timestamp == rtp_timestamp)
        {
            return;
        }
        if self.frame_playout_base_times.len() >= FRAME_PLAYOUT_BASE_TRACK_CAPACITY {
            self.frame_playout_base_times.pop_front();
        }
        self.frame_playout_base_times
            .push_back((rtp_timestamp, observed_at));
    }

    fn take_frame_first_packet_arrived_at(
        &mut self,
        rtp_timestamp: u32,
    ) -> Option<std::time::Instant> {
        let index = self
            .frame_playout_base_times
            .iter()
            .position(|(timestamp, _)| *timestamp == rtp_timestamp)?;
        self.frame_playout_base_times
            .remove(index)
            .map(|(_, observed_at)| observed_at)
    }

    fn record_frame_first_packet_sequence(&mut self, rtp_timestamp: u32, sequence: u16) {
        if self
            .frame_first_packet_sequences
            .iter()
            .any(|(timestamp, _)| *timestamp == rtp_timestamp)
        {
            return;
        }
        if self.frame_first_packet_sequences.len() >= FRAME_PLAYOUT_BASE_TRACK_CAPACITY {
            self.frame_first_packet_sequences.pop_front();
        }
        self.frame_first_packet_sequences
            .push_back((rtp_timestamp, sequence));
    }

    fn take_frame_first_packet_sequence(&mut self, rtp_timestamp: u32) -> Option<u16> {
        let index = self
            .frame_first_packet_sequences
            .iter()
            .position(|(timestamp, _)| *timestamp == rtp_timestamp)?;
        self.frame_first_packet_sequences
            .remove(index)
            .map(|(_, sequence)| sequence)
    }

    fn response_oos_depth_p75(&self) -> Option<u16> {
        if self.recent_oos_depths.is_empty() {
            return None;
        }
        let mut samples: Vec<u16> = self.recent_oos_depths.iter().copied().collect();
        samples.sort_unstable();
        let p75_index = ((samples.len().saturating_sub(1) * 3) / 4).min(samples.len() - 1);
        Some(samples[p75_index])
    }

    fn build_ingress_materialization_fallback_budget(
        &self,
        frame_value: FrameValue,
        frame_playout_deadline_at_ms: Option<f64>,
        frame_unrecoverable_reason: Option<&str>,
    ) -> FrameBudgetContext {
        let window_source = if frame_playout_deadline_at_ms.is_some() {
            FrameBudgetWindowSource::Recovery
        } else {
            FrameBudgetWindowSource::Playout
        };
        let cloud_rtt_ms = self.cloud_nack_rtt_ms();
        let now_ms = now_ms_f64();
        let estimated_recovery_arrival_ms = if cloud_rtt_ms > 0.0 {
            Some(now_ms + cloud_rtt_ms)
        } else {
            None
        };
        let fallback = FrameBudgetContext::for_transport(
            frame_value,
            self.is_blocking_non_keyframe_admission(),
            Some(cloud_rtt_ms),
            estimated_recovery_arrival_ms,
            frame_playout_deadline_at_ms,
            self.is_cloud_startup_transport_profile(),
            window_source,
        );
        // RTT 不可用时（Unknown），for_transport 构造的 context 在 rtt_slack 维度上
        // 与 for_ingress_materialization_parts 相同，但其他字段（window_source、failure_cost）
        // 可能因 waiting_keyframe 状态而不同，会引入非预期的 ratio 偏移。
        // 退回到 for_ingress_materialization_parts 以保持与原有行为一致，避免在
        // RTT 信息缺失时引入额外的不确定性。
        if matches!(fallback.rtt_slack, FrameBudgetRttSlack::Unknown) {
            return FrameBudgetContext::for_ingress_materialization_parts(
                frame_value,
                frame_playout_deadline_at_ms,
                frame_unrecoverable_reason,
            );
        }
        fallback
    }

    fn nack_maintenance_timeout(&self, base_timeout: std::time::Duration) -> std::time::Duration {
        let elapsed = self.last_nack_maintenance_tick_at.elapsed();
        let until_tick = if elapsed >= self.nack_maintenance_tick_interval {
            std::time::Duration::ZERO
        } else {
            self.nack_maintenance_tick_interval - elapsed
        };
        base_timeout.min(until_tick)
    }

    pub(super) fn should_run_nack_maintenance_tick(&self) -> bool {
        self.last_nack_maintenance_tick_at.elapsed() >= self.nack_maintenance_tick_interval
    }

    pub(super) fn maybe_retry_waiting_recovery_keyframe(&mut self, now_ms: f64) {
        if !self.is_blocking_non_keyframe_admission() {
            return;
        }
        let should_retry = self
            .next_recovery_keyframe_retry_at_ms
            .is_some_and(|next_retry_at_ms| now_ms >= next_retry_at_ms);
        if !should_retry {
            return;
        }
        // 超过重试上限后停止发送请求，避免服务端无响应时无限重试。
        if self.recovery_keyframe_retry_count >= RECOVERY_KEYFRAME_RETRY_MAX_COUNT {
            crate::xbx_log_warn!(
                "[RtcVideoFrameSource] recovery keyframe retry limit reached count={} waiting_since_ms={:?}",
                self.recovery_keyframe_retry_count,
                self.waiting_recovery_keyframe_since_ms
            );
            // 清空 next_retry_at_ms，不再触发后续重试。
            self.next_recovery_keyframe_retry_at_ms = None;
            return;
        }
        self.request_recovery_keyframe_from_source(
            "chain-recovery-keyframe-timeout-retry",
            None,
            now_ms,
        );
        self.recovery_keyframe_retry_count = self.recovery_keyframe_retry_count.saturating_add(1);
        self.next_recovery_keyframe_retry_at_ms =
            Some(now_ms + self.recovery_keyframe_retry_interval_ms);
        crate::xbx_log_warn!(
            "[RtcVideoFrameSource] recovery keyframe wait timeout retry count={} waiting_since_ms={:?}",
            self.recovery_keyframe_retry_count,
            self.waiting_recovery_keyframe_since_ms
        );
    }

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
            || !self.is_blocking_non_keyframe_admission()
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
        let historical_output_seen = stats.latest_video_decode_ok_time_ms.is_some()
            || stats.latest_video_host_present_time_ms.is_some()
            || stats.host_mailbox_enqueue_count_total > 0
            || stats.host_frame_present_epoch > 0;
        if !stats.transport_recovery_episode_active {
            return historical_output_seen;
        }
        let has_current_clean_anchor = stats.video_anchor_clean_epoch
            == Some(stats.transport_recovery_epoch)
            && stats.video_anchor_clean_source_event.as_deref()
                == Some("chain-clean-anchor-submitted");
        if has_current_clean_anchor {
            return true;
        }
        let Some(recovery_opened_at_ms) = stats.transport_recovery_episode_opened_at_ms else {
            return historical_output_seen;
        };
        stats
            .latest_video_decode_ok_time_ms
            .is_some_and(|at_ms| at_ms >= recovery_opened_at_ms)
            || stats
                .latest_host_mailbox_submit_time_ms
                .is_some_and(|at_ms| at_ms >= recovery_opened_at_ms)
            || stats
                .latest_video_host_present_time_ms
                .is_some_and(|at_ms| at_ms >= recovery_opened_at_ms)
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
                    | H264BootstrapRejectReason::BootstrapMissingIdr
                    | H264BootstrapRejectReason::NonIdrVcl
                    | H264BootstrapRejectReason::MixedIdrWithTrailingDelta
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

    pub(super) fn request_recovery_keyframe_from_source(
        &mut self,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) {
        self.timeline_state.on_recovery_keyframe_requested();
        self.record_video_timeline_observation(source_event, None, frame_rtp_timestamp, now_ms);
        self.queue_transport_observation(TransportObservation::Loss(
            TransportLossObservation::RecoveryKeyframeRequested,
        ));
    }

    pub(super) fn request_recovery_keyframe_soft_from_source(
        &mut self,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) {
        self.timeline_state.on_recovery_keyframe_requested_soft();
        self.record_video_timeline_observation(source_event, None, frame_rtp_timestamp, now_ms);
        self.queue_transport_observation(TransportObservation::Loss(
            TransportLossObservation::RecoveryKeyframeRequested,
        ));
    }

    fn enter_recovery_wait_from_source(
        &mut self,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        candidate_state: XbxEngineAnchorCandidateState,
        failure_reason: Option<XbxEngineAnchorCandidateFailureReason>,
        timeline_reason: &'static str,
        requested_frame_value: RecoveryFrameValue,
        invalid_bootstrap_metadata_ready: bool,
        now_ms: f64,
    ) {
        let should_soft_request = self
            .runtime_stats
            .read(|stats| {
                let invalid_bootstrap_reason = match timeline_reason {
                    "bootstrapMissingIdr" | "mixedIdrWithTrailingDelta" => Some(timeline_reason),
                    "bootstrapMissingSps"
                    | "bootstrapMissingPps"
                    | "inspectionRejectInvalidSliceHeader" => Some(timeline_reason),
                    _ => None,
                };
                Self::should_soft_request_recovery_keyframe(
                    stats,
                    now_ms,
                    invalid_bootstrap_reason,
                    invalid_bootstrap_metadata_ready,
                    !matches!(requested_frame_value, RecoveryFrameValue::RecoveryAnchor),
                    false,
                )
            })
            .unwrap_or(false);
        if should_soft_request {
            self.request_recovery_keyframe_soft_from_source(
                source_event,
                frame_rtp_timestamp,
                now_ms,
            );
            self.record_anchor_candidate_ledger(
                frame_rtp_timestamp,
                source_event,
                candidate_state,
                failure_reason,
                now_ms,
            );
            return;
        }
        self.set_is_blocking_non_keyframe_admission(true);
        self.timeline_state
            .on_admission_await_recovery_keyframe(Some(timeline_reason));
        self.record_video_timeline_observation(source_event, None, frame_rtp_timestamp, now_ms);
        self.record_anchor_candidate_ledger(
            frame_rtp_timestamp,
            source_event,
            candidate_state,
            failure_reason,
            now_ms,
        );
        self.queue_transport_observation(TransportObservation::Admission(
            TransportAdmissionObservation::AwaitRecoveryKeyframe,
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
            && clean_anchor_source_event == Some("chain-clean-anchor-submitted")
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
        if self.timeline_state.chain_requires_recovery_anchor() {
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

    fn on_nack_window_add_outcome(
        &mut self,
        outcome: NackWindowAddOutcome,
        rtp_timestamp: u32,
        now_ms: f64,
    ) {
        if outcome.is_oos {
            self.oos_event_count = self.oos_event_count.saturating_add(1);
            self.recent_oos_active_until_ms = Some(now_ms + OOS_ACTIVITY_COOLDOWN_MS);
            self.mark_frame_oos(rtp_timestamp);
            let frame_has_oos = self.frame_seen_oos(rtp_timestamp);
            let recent_oos_active = self.oos_recently_active(now_ms);
            if let Some(distance) = outcome.oos_distance_from_end {
                if self.recent_oos_depths.len() >= OOS_DEPTH_WINDOW_CAPACITY {
                    self.recent_oos_depths.pop_front();
                }
                self.recent_oos_depths.push_back(distance);
                self.update_dynamic_nack_skip_last_n(now_ms);
            }
            if self.oos_event_count == 1 || self.oos_event_count.is_power_of_two() {
                crate::xbx_log_info!(
                    "[RtcVideoFrameSource] oos event seq={} distance={:?} skip_last_n={}",
                    outcome.seq,
                    outcome.oos_distance_from_end,
                    self.nack_skip_last_n
                );
                crate::xbx_log_info!(
                    "[RtcVideoFrameSource] oos signal frame_ts={} frame_has_oos={} recent_active={}",
                    rtp_timestamp,
                    frame_has_oos,
                    recent_oos_active
                );
            }
        }

        if outcome.overflow_advanced {
            self.nack_window_overflow_count = self.nack_window_overflow_count.saturating_add(1);
            if let Some(range) = outcome.overflow_pruned_range {
                self.prune_pending_nack_for_window_range(range, now_ms);
            }
        }
    }

    fn update_dynamic_nack_skip_last_n(&mut self, now_ms: f64) {
        if self
            .last_nack_skip_last_n_updated_at_ms
            .is_some_and(|last_ms| {
                (now_ms - last_ms).max(0.0) < OOS_SKIP_LAST_N_REFRESH_INTERVAL_MS
            })
        {
            return;
        }
        self.last_nack_skip_last_n_updated_at_ms = Some(now_ms);
        if self.recent_oos_depths.is_empty() {
            self.nack_skip_last_n = 2;
            return;
        }
        let mut samples: Vec<u16> = self.recent_oos_depths.iter().copied().collect();
        samples.sort_unstable();
        let last_index = samples.len() - 1;
        let p50 = samples[(last_index / 2).min(last_index)];
        let p75 = samples[((last_index * 3) / 4).min(last_index)];
        let p90 = samples[((last_index * 9) / 10).min(last_index)];
        self.nack_skip_last_n = if p90 >= 6 {
            6
        } else if p75 >= 4 {
            4
        } else if p50 <= 2 {
            2
        } else {
            4
        };
    }

    fn prune_pending_nack_for_window_range(&mut self, range: SequenceRange, now_ms: f64) {
        let removed = self
            .nack_scheduler
            .prune_pending_in_range(range.start, range.end_exclusive);
        if removed.is_empty() {
            return;
        }
        crate::xbx_log_info!(
            "[RtcVideoFrameSource] nack window overflow pruned pending={} range={}..{} total={}",
            removed.len(),
            range.start,
            range.end_exclusive,
            self.nack_window_overflow_count
        );
        if let Some(first) = removed.first().copied() {
            self.record_video_timeline_observation(
                "gap-overflow-pruned",
                Some(first),
                None,
                now_ms,
            );
        }
    }

    fn mark_frame_oos(&mut self, rtp_timestamp: u32) {
        if let Some((_, flag)) = self
            .frame_oos_flags
            .iter_mut()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
        {
            *flag = true;
            return;
        }
        if self.frame_oos_flags.len() >= FRAME_OOS_TRACK_CAPACITY {
            self.frame_oos_flags.pop_front();
        }
        self.frame_oos_flags.push_back((rtp_timestamp, true));
    }

    pub(super) fn frame_seen_oos(&self, rtp_timestamp: u32) -> bool {
        self.frame_oos_flags
            .iter()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
            .is_some_and(|(_, flag)| *flag)
    }

    pub(super) fn oos_recently_active(&self, now_ms: f64) -> bool {
        self.recent_oos_active_until_ms
            .is_some_and(|until_ms| now_ms <= until_ms)
    }

    pub(super) async fn recv_frame_inner(&mut self) -> Option<AssembledVideoFrame> {
        loop {
            self.maybe_ack_clean_anchor_commit_from_runtime_stats();
            if self.should_run_nack_maintenance_tick() {
                self.maybe_run_nack_maintenance().await;
            }
            self.maybe_emit_jitter_early_boundary();
            if let Some(sample) = self.sample_builder.pop() {
                if let Some(boundary) = self.pending_marker_boundary {
                    let wait_ms = boundary.observed_at.elapsed().as_millis();
                    crate::xbx_log_debug!(
                        "[RtcVideoFrameSource] marker->sample wait ts={} wait_ms={}",
                        sample.packet_timestamp,
                        wait_ms
                    );
                    self.pending_marker_boundary = None;
                }
                self.clear_pending_timeout_confirmations();
                self.last_packet_time = std::time::Instant::now();
                self.assembling_frame_start = None;
                self.current_assembly_packet_count = 0;
                let mut payload = sample.data.to_vec();
                self.assembled_frame_count = self.assembled_frame_count.saturating_add(1);
                self.maybe_request_first_frame_acquisition_keyframe(
                    Some(sample.packet_timestamp),
                    FirstFrameAcquisitionRequestKind::Initial,
                );
                self.maybe_seed_h264_bootstrap_from_remote_answer();
                let mut inspection = match self.h264_inspector.inspect_access_unit(&payload) {
                    Ok(inspection) => inspection,
                    Err(error) => {
                        let now_ms = now_ms_f64();
                        crate::xbx_log_error!(
                            "[RtcVideoFrameSource] h264 inspection failed: {error}"
                        );
                        // 检查错误：保守处理为 disposable
                        self.timeline_state.observe_frame(
                            sample.packet_timestamp,
                            now_ms,
                            None,
                            "disposable",
                        );
                        self.timeline_state.mark_frame_closed(
                            sample.packet_timestamp,
                            now_ms,
                            None,
                            "disposable",
                            Some("inspectionError"),
                        );
                        self.enter_recovery_wait_from_source(
                            "frame-inspection-error-await-anchor",
                            Some(sample.packet_timestamp),
                            XbxEngineAnchorCandidateState::Rejected,
                            Some(XbxEngineAnchorCandidateFailureReason::Unknown),
                            "inspectionError",
                            RecoveryFrameValue::RecoveryAnchor,
                            false,
                            now_ms,
                        );
                        continue;
                    }
                };
                self.runtime_stats.update(|stats| {
                    stats.recovery_codec_bootstrap_salvage_applied = None;
                    stats.recovery_codec_bootstrap_salvage_failed_reason = None;
                });
                if let Some(salvaged) =
                    try_h264_bootstrap_ps_salvage_au(&self.h264_inspector, &inspection, &payload)
                {
                    match self.h264_inspector.inspect_access_unit(&salvaged) {
                        Ok(new_insp) if new_insp.bootstrap_ready => {
                            inspection = new_insp;
                            payload = salvaged;
                            self.runtime_stats.update(|stats| {
                                stats.recovery_codec_bootstrap_salvage_applied = Some(true);
                                stats.recovery_codec_bootstrap_salvage_failed_reason = None;
                            });
                        }
                        Ok(_) => {
                            self.runtime_stats.update(|stats| {
                                stats.recovery_codec_bootstrap_salvage_applied = Some(false);
                                stats.recovery_codec_bootstrap_salvage_failed_reason =
                                    Some("salvageBootstrapStillNotReady".into());
                            });
                        }
                        Err(_) => {
                            self.runtime_stats.update(|stats| {
                                stats.recovery_codec_bootstrap_salvage_applied = Some(false);
                                stats.recovery_codec_bootstrap_salvage_failed_reason =
                                    Some("salvageReinspectFailed".into());
                            });
                        }
                    }
                }
                let inspection_now_ms = now_ms_f64();
                let first_frame_acquired = self
                    .runtime_stats
                    .read(|stats| Self::first_frame_acquired(stats))
                    .unwrap_or(false);
                let is_blocking_non_keyframe_admission = self.is_blocking_non_keyframe_admission();
                let clean_anchor_building_phase_active = self
                    .timeline_state
                    .recovery_chain_building_phase_active(inspection_now_ms, "disposable");
                let prior_output_continuation_allowed = prior_output_continuation_allowed(
                    first_frame_acquired,
                    is_blocking_non_keyframe_admission,
                    self.timeline_state.chain_requires_recovery_anchor(),
                    clean_anchor_building_phase_active,
                );
                let decoder_bootstrap_no_output_continuation_allowed = self
                    .decoder_bootstrap_no_output_continuation_allowed(
                        &inspection,
                        first_frame_acquired,
                        inspection_now_ms,
                    );
                // clean anchor 已成立后，恢复保活期里的健康 continuation 不能再被“尚未首帧出图”
                // 这条旧 bootstrap 语义重新打回等待关键帧。
                let sustaining_recovery_continuation_allowed =
                    self.timeline_state.in_sustaining_recovery()
                        && inspection.slice_headers_valid
                        && inspection.delta_continuation_ready()
                        && inspection.committed_sps_present()
                        && inspection.committed_pps_present();
                let inspection_admission = resolve_inspection_admission(
                    &inspection,
                    prior_output_continuation_allowed,
                    decoder_bootstrap_no_output_continuation_allowed,
                    sustaining_recovery_continuation_allowed,
                );
                let admission_accepted =
                    matches!(inspection_admission, InspectionAdmission::Accept);
                let response_detail =
                    keyframe_episode_response_detail(&inspection, inspection_admission);
                let continuation_verdict =
                    Self::h264_continuation_verdict(&inspection, inspection_admission);
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
                        continuation_verdict,
                        admission_accepted,
                        observed_at_ms: inspection_now_ms,
                        ..Default::default()
                    },
                );
                if self.should_request_first_frame_acquisition_followup_keyframe(&inspection) {
                    self.maybe_request_first_frame_acquisition_keyframe(
                        Some(sample.packet_timestamp),
                        FirstFrameAcquisitionRequestKind::Followup,
                    );
                }
                let media_dropped_packets = sample
                    .prev_dropped_packets
                    .saturating_sub(sample.prev_padding_packets);
                match inspection_admission {
                    InspectionAdmission::Accept => {}
                    InspectionAdmission::AwaitRecoveryKeyframe => {
                        if media_dropped_packets > 0
                            && (matches!(
                                inspection.bootstrap_reject_reason,
                                Some(H264BootstrapRejectReason::InvalidSliceHeader)
                            ) || !inspection.slice_headers_valid)
                        {
                            self.mark_frame_head_missing_signal(sample.packet_timestamp);
                        }
                        let now_ms = now_ms_f64();
                        let reject_reason = inspection_bootstrap_reason(&inspection);
                        crate::xbx_log_warn!(
                            "[RtcVideoFrameSource] h264 inspection rejected sample ts={} bootstrap={:?} slice_headers_valid={}",
                            sample.packet_timestamp,
                            inspection.bootstrap_reject_reason,
                            inspection.slice_headers_valid
                        );
                        let sustaining_recovery_failed =
                            self.timeline_state.in_sustaining_recovery();
                        if sustaining_recovery_failed {
                            self.set_is_blocking_non_keyframe_admission(true);
                            self.timeline_state
                                .on_sustaining_recovery_failed(reject_reason);
                        } else {
                            self.set_is_blocking_non_keyframe_admission(true);
                            self.timeline_state
                                .on_admission_await_recovery_keyframe(Some(reject_reason));
                        }
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
                        let rejection_source_event = if sustaining_recovery_failed {
                            "frame-inspection-rejected-trigger-recovery-anchor"
                        } else {
                            "frame-inspection-rejected-await-anchor"
                        };
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
                        if sustaining_recovery_failed {
                            self.record_video_timeline_observation(
                                rejection_source_event,
                                None,
                                Some(sample.packet_timestamp),
                                now_ms,
                            );
                            self.record_anchor_candidate_ledger(
                                Some(sample.packet_timestamp),
                                rejection_source_event,
                                XbxEngineAnchorCandidateState::Rejected,
                                Some(failure_reason),
                                now_ms,
                            );
                            let should_soft_request = self
                                .runtime_stats
                                .read(|stats| {
                                    Self::should_soft_request_recovery_keyframe(
                                        stats,
                                        now_ms,
                                        match reject_reason {
                                            "bootstrapMissingIdr" | "mixedIdrWithTrailingDelta" => {
                                                Some(reject_reason)
                                            }
                                            "bootstrapMissingSps"
                                            | "bootstrapMissingPps"
                                            | "inspectionRejectInvalidSliceHeader" => {
                                                Some(reject_reason)
                                            }
                                            _ => None,
                                        },
                                        inspection.committed_sps_present()
                                            && inspection.committed_pps_present()
                                            && inspection.delta_continuation_ready(),
                                        true,
                                        true,
                                    )
                                })
                                .unwrap_or(false);
                            if should_soft_request {
                                self.request_recovery_keyframe_soft_from_source(
                                    "chain-recovery-anchor-requested",
                                    Some(sample.packet_timestamp),
                                    now_ms,
                                );
                            } else {
                                self.request_recovery_keyframe_from_source(
                                    "chain-recovery-anchor-requested",
                                    Some(sample.packet_timestamp),
                                    now_ms,
                                );
                            }
                        } else {
                            self.enter_recovery_wait_from_source(
                                rejection_source_event,
                                Some(sample.packet_timestamp),
                                XbxEngineAnchorCandidateState::Rejected,
                                Some(failure_reason),
                                reject_reason,
                                RecoveryFrameValue::RecoveryAnchor,
                                inspection.committed_sps_present()
                                    && inspection.committed_pps_present()
                                    && inspection.delta_continuation_ready(),
                                now_ms,
                            );
                        }
                        continue;
                    }
                }
                let is_keyframe = inspection.is_idr;
                let config_changed = inspection.config_changed;
                if media_dropped_packets > 0 {
                    self.record_frame_drop_attribution(
                        sample.packet_timestamp,
                        media_dropped_packets,
                    );
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
                // media_type_label: 基于 H.264 inspection 的媒体类型标签，用于 timeline 追踪和 NACK 策略
                // 注意：这与 FrameBudgetContext.link_value_label() 不同，后者是恢复价值分档
                let media_type_label = if is_keyframe {
                    "keyframe"
                } else if config_changed {
                    "reference"
                } else {
                    "delta"
                };
                let healthy_delta_continuation = !is_keyframe
                    && media_type_label == "delta"
                    && is_recovery_delta_continuation_ready(&inspection);
                if healthy_delta_continuation {
                    let _ = self
                        .timeline_state
                        .reopen_delta_continuation_after_clean_anchor(frame_now_ms);
                }
                let hard_recovery_gap_risk = self.timeline_state.has_hard_recovery_gap_risk();
                let (next_is_blocking_non_keyframe_admission, recovery_action) =
                    resolve_recovery_keyframe_action(
                        first_frame_acquired,
                        is_blocking_non_keyframe_admission,
                        self.timeline_state.in_sustaining_recovery() && healthy_delta_continuation,
                        hard_recovery_gap_risk,
                        clean_anchor_building_phase_active && healthy_delta_continuation,
                        self.sample_loss_burst_count,
                        media_dropped_packets,
                        is_keyframe,
                    );

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

                match recovery_action {
                    RecoveryKeyframeAction::Submit => {
                        self.set_is_blocking_non_keyframe_admission(
                            next_is_blocking_non_keyframe_admission,
                        );
                    }
                    RecoveryKeyframeAction::DropAndRequestPli => {
                        self.set_is_blocking_non_keyframe_admission(
                            next_is_blocking_non_keyframe_admission,
                        );
                        self.handle_drop_and_request_keyframe_action(
                            sample.packet_timestamp,
                            media_dropped_packets,
                            is_keyframe,
                            media_type_label,
                        )
                        .await;
                        continue;
                    }
                    RecoveryKeyframeAction::WaitKeyframe => {
                        self.set_is_blocking_non_keyframe_admission(
                            next_is_blocking_non_keyframe_admission,
                        );
                        let now_ms = now_ms_f64();
                        self.enter_recovery_wait_from_source(
                            "frame-await-recovery-anchor",
                            Some(sample.packet_timestamp),
                            XbxEngineAnchorCandidateState::AwaitingRecovery,
                            Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
                            "awaitingRecoveryAnchor",
                            if media_type_label == "reference" {
                                RecoveryFrameValue::Reference
                            } else {
                                RecoveryFrameValue::Continuity
                            },
                            false,
                            now_ms,
                        );
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
                // 将媒体语义标签转换为恢复语义标签
                let recovery_importance = match media_type_label {
                    "keyframe" => "anchor",
                    "reference" => "supply",
                    "delta" => "disposable",
                    _ => "disposable",
                };
                self.timeline_state.observe_frame(
                    sample.packet_timestamp,
                    frame_now_ms,
                    Some(is_keyframe),
                    recovery_importance,
                );
                self.record_video_timeline_observation(
                    "frame-observed",
                    None,
                    Some(sample.packet_timestamp),
                    frame_now_ms,
                );
                let assembled_at = std::time::Instant::now();
                // 优先取首包到达时刻作为 playout 基准，减少 SampleBuilder 等待引入的固有延迟。
                // 找不到记录时保持 None，物化层会 fallback 到 assembled_at，语义上有区别。
                let first_packet_arrived_at =
                    self.take_frame_first_packet_arrived_at(sample.packet_timestamp);
                let first_packet_sequence =
                    self.take_frame_first_packet_sequence(sample.packet_timestamp);
                self.transport_deadline_tracker
                    .record_frame_arrival(now_ms_f64());
                let (
                    frame_playout_deadline_at_ms,
                    mut frame_recovery_disposition,
                    frame_unrecoverable_reason,
                    ledger_budget_context,
                ) = self.take_frame_recovery_ledger(sample.packet_timestamp);
                let mut frame_budget = ledger_budget_context.unwrap_or_else(|| {
                    self.build_ingress_materialization_fallback_budget(
                        frame_value,
                        frame_playout_deadline_at_ms,
                        frame_unrecoverable_reason.as_deref(),
                    )
                });
                self.runtime_stats
                    .record_picture_recovery_episode_response_observed(
                        inspection_now_ms,
                        Some(sample.packet_timestamp),
                        inspection.is_idr,
                        response_detail,
                        first_packet_sequence,
                        self.response_oos_depth_p75(),
                        self.frame_seen_head_missing(sample.packet_timestamp)
                            || self.head_missing_recently_active(inspection_now_ms),
                        matches!(
                            frame_unrecoverable_reason.as_deref(),
                            Some("deadlineExceeded" | "deadlineExceededBeforeAdmission")
                        ),
                    );
                self.ingress_budget_materialized_count =
                    self.ingress_budget_materialized_count.saturating_add(1);
                if ledger_budget_context.is_none() {
                    self.ingress_budget_fallback_count =
                        self.ingress_budget_fallback_count.saturating_add(1);
                    if matches!(frame_budget.rtt_slack, FrameBudgetRttSlack::Unknown) {
                        self.ingress_budget_unknown_rtt_count =
                            self.ingress_budget_unknown_rtt_count.saturating_add(1);
                    }
                    if self.ingress_budget_fallback_count == 1
                        || self.ingress_budget_fallback_count.is_power_of_two()
                    {
                        let unknown_ratio = if self.ingress_budget_fallback_count == 0 {
                            0.0
                        } else {
                            (self.ingress_budget_unknown_rtt_count as f64 * 100.0)
                                / (self.ingress_budget_fallback_count as f64)
                        };
                        crate::xbx_log_info!(
                            "[RtcVideoFrameSource] ingress fallback budget count={} unknown_rtt_count={} unknown_ratio={:.1}% rtt_slack={:?} base_source={}",
                            self.ingress_budget_fallback_count,
                            self.ingress_budget_unknown_rtt_count,
                            unknown_ratio,
                            frame_budget.rtt_slack,
                            if first_packet_arrived_at.is_none() {
                                "assembledAt"
                            } else {
                                "firstPacketAt"
                            }
                        );
                    }
                }
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
                // 恢复期首个干净 IDR 自身就是新的 decoder success edge，
                // 允许它直接带着 clean-anchor 提交资格穿过 sustaining exit gate。
                let current_frame_allows_sustaining_exit = inspection.is_idr
                    && admission_accepted
                    && inspection.bootstrap_ready
                    && media_dropped_packets == 0;
                let can_exit_sustaining_recovery = !self.timeline_state.in_sustaining_recovery()
                    || self.decoder_feedback_allows_sustaining_exit(complete_candidate_now_ms)
                    || current_frame_allows_sustaining_exit;
                let clean_anchor_complete_candidate =
                    can_exit_sustaining_recovery && current_frame_allows_sustaining_exit;
                if can_exit_sustaining_recovery {
                    // 将媒体语义标签转换为恢复语义标签
                    let recovery_importance = match media_type_label {
                        "keyframe" => "anchor",
                        "reference" => "supply",
                        "delta" => "disposable",
                        _ => "disposable",
                    };
                    self.timeline_state.mark_frame_complete_candidate(
                        sample.packet_timestamp,
                        complete_candidate_now_ms,
                        Some(is_keyframe),
                        recovery_importance,
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
                    if clean_anchor_complete_candidate {
                        let should_rearm_clean_anchor = self
                            .runtime_stats
                            .read(Self::should_rearm_clean_anchor_for_transport_await)
                            .unwrap_or(false);
                        let needs_recovery_anchor =
                            self.timeline_state.chain_requires_recovery_anchor()
                                || matches!(
                                    self.timeline_state.chain_state(),
                                    ChainState::Broken | ChainState::Recovering
                                )
                                || should_rearm_clean_anchor;
                        if needs_recovery_anchor {
                            self.timeline_state
                                .on_clean_anchor_ingress(sample.packet_timestamp, frame_now_ms);
                        }
                    }
                } else {
                    self.record_video_timeline_observation(
                        "frame-complete-candidate-decode-feedback-blocked",
                        None,
                        Some(sample.packet_timestamp),
                        complete_candidate_now_ms,
                    );
                }

                let can_commit_clean_anchor_now = clean_anchor_complete_candidate;
                let recovery_epoch_tag = self
                    .runtime_stats
                    .read(|stats| stats.transport_recovery_epoch);
                let recovery_owner_rtp_timestamp = self
                    .runtime_stats
                    .read(|stats| {
                        stats
                            .latest_keyframe_request_episode
                            .as_ref()
                            .and_then(|episode| episode.response_rtp_timestamp)
                    })
                    .flatten();
                let clean_anchor_commit_recovery_epoch = if can_commit_clean_anchor_now {
                    self.timeline_state
                        .peek_clean_anchor_stats_commit_candidate_if_stable(
                            sample.packet_timestamp,
                            complete_candidate_now_ms,
                        )
                        .and_then(|committed_ts| {
                            if committed_ts != sample.packet_timestamp {
                                return None;
                            }
                            self.runtime_stats
                                .read(|stats| stats.transport_recovery_epoch)
                        })
                } else {
                    None
                };
                let is_recovery_owner_frame =
                    recovery_owner_rtp_timestamp == Some(sample.packet_timestamp);
                let eligible_recovery_owner_frame = frame_unrecoverable_reason.is_none()
                    && (clean_anchor_commit_recovery_epoch.is_some()
                        || (is_recovery_owner_frame && current_frame_allows_sustaining_exit))
                    && inspection.is_idr;
                if eligible_recovery_owner_frame {
                    frame_budget = frame_budget.promote_to_recovery_window(frame_value);
                    if matches!(frame_recovery_disposition, FrameRecoveryDisposition::Steady) {
                        frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
                    }
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
                    recovery_epoch_tag,
                    recovery_owner_rtp_timestamp,
                    clean_anchor_commit_recovery_epoch,
                    first_packet_sequence,
                    frame_recovery_disposition,
                    frame_unrecoverable_reason,
                    assembled_at,
                    first_packet_arrived_at,
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
                let sustaining_recovery_failed = self.timeline_state.in_sustaining_recovery();
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
                    if sustaining_recovery_failed {
                        let should_soft_request = self
                            .runtime_stats
                            .read(|stats| {
                                Self::should_soft_request_recovery_keyframe(
                                    stats,
                                    timeout_now_ms,
                                    None,
                                    false,
                                    false,
                                    true,
                                )
                            })
                            .unwrap_or(false);
                        if should_soft_request {
                            self.request_recovery_keyframe_soft_from_source(
                                "chain-recovery-anchor-requested",
                                None,
                                timeout_now_ms,
                            );
                        } else {
                            self.request_recovery_keyframe_from_source(
                                "chain-recovery-anchor-requested",
                                None,
                                timeout_now_ms,
                            );
                        }
                    } else {
                        self.queue_transport_observation(if thin_stream_stall {
                            TransportObservation::StreamThinStall
                        } else {
                            TransportObservation::StreamIdleTimeout
                        });
                    }
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
            let read_timeout = self.nack_maintenance_timeout(read_timeout);
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
                    self.remember_frame_playout_base_candidate(
                        rtp.header.timestamp,
                        self.last_packet_time,
                    );
                    if self.assembling_frame_start.is_none() {
                        self.assembling_frame_start = Some(self.last_packet_time);
                        self.current_assembly_packet_count = 0;
                    }
                    self.current_assembly_packet_count =
                        self.current_assembly_packet_count.saturating_add(1);
                    let seq = rtp.header.sequence_number;
                    let now_ms = now_ms_f64();
                    let add_outcome = self.nack_window.add(seq);
                    self.on_nack_window_add_outcome(add_outcome, rtp.header.timestamp, now_ms);

                    // 更新帧边界追踪状态
                    let is_priority = super::sink::is_likely_h264_recovery_priority(&rtp.payload);
                    if let Ok(mut tracker) = self.frame_boundary.lock() {
                        tracker.on_packet_arrived(
                            seq,
                            rtp.header.timestamp,
                            rtp.header.marker,
                            is_priority,
                        );
                    }

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
                            gap_transport_evidence(resolved.frame_is_keyframe),
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
                    let (next_highest_sequence, forward_gap) =
                        detect_forward_gap(self.last_highest_rtp_sequence, seq);
                    self.last_highest_rtp_sequence = next_highest_sequence;
                    if let Some((expected_sequence, received_sequence)) = forward_gap {
                        let missing_sequences = super::nack::wrapping_sequence_range(
                            expected_sequence,
                            received_sequence,
                        );
                        // 前向 gap：匿名缺洞保守处理为 disposable
                        self.timeline_state.observe_gap(
                            &missing_sequences,
                            now_ms,
                            Some(rtp.header.timestamp),
                            "disposable",
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
                    self.maybe_run_nack_maintenance().await;
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
                    self.record_frame_first_packet_sequence(
                        rtp.header.timestamp,
                        rtp.header.sequence_number,
                    );
                    if rtp.header.marker {
                        self.jitter_marker_seen_count =
                            self.jitter_marker_seen_count.saturating_add(1);
                        self.pending_marker_boundary = Some(super::PendingMarkerBoundary {
                            sequence: rtp.header.sequence_number,
                            rtp_timestamp: rtp.header.timestamp,
                            media_payload_type: rtp.header.payload_type,
                            observed_at: std::time::Instant::now(),
                        });
                        if self.jitter_marker_seen_count == 1
                            || self.jitter_marker_seen_count.is_power_of_two()
                        {
                            crate::xbx_log_debug!(
                                "[RtcVideoFrameSource] marker observed count={} seq={} ts={} early_emit={}",
                                self.jitter_marker_seen_count,
                                rtp.header.sequence_number,
                                rtp.header.timestamp,
                                self.jitter_early_emit_enabled
                            );
                        }
                    }
                    self.sample_builder.push(rtp);
                }
                Ok(None) => {
                    let cause = self.runtime_stats.current_video_ingress_close_cause();
                    let now_ms = now_ms_f64();
                    self.runtime_stats
                        .record_video_ingress_rx_closed(now_ms, cause.as_deref());
                    crate::xbx_log_error!(
                        "[RtcVideoFrameSource] rx closed cause={}",
                        cause.as_deref().unwrap_or("upstreamSenderDropped")
                    );
                    return None;
                }
                Err(_) => {
                    self.maybe_run_nack_maintenance().await;
                }
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
