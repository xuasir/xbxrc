//! pre-decode AU 裁决（`RtcVideoFrameSource` 子模块，可访问 ingress 私有字段）。

use base64::Engine as _;
use bytes::Bytes;

use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264BootstrapRejectReason};
use crate::media::video::ingress::budget::{
    FrameBudgetContext, FrameBudgetRttSlack, FrameBudgetWindowSource,
};
use crate::media::video::types::{
    AssembledVideoFrame, FrameRecoveryDisposition, FrameValue, VideoCodec,
};
use crate::transport::rtc::receive::decode_gate_eval::{
    FirstFrameAcquisitionRequestKind, FirstFrameAcquisitionRuntimeContext, RecoveryKeyframeAction,
};
use crate::transport::rtc::receive::nack_policy::RECOVERY_KEYFRAME_RETRY_MAX_COUNT;
use crate::transport::rtc::receive::{
    insert_decision_label, insert_decision_to_inspection_admission,
    inspection_bootstrap_blocks_delta_continuation, inspection_bootstrap_reason,
    keyframe_episode_response_detail, now_ms_f64, receiver_state_blocks_delta_continuation,
    recovery_keyframe_action_for_insert_decision, resolve_insert_decision_with_reason,
    should_block_non_keyframe_admission, DecodeCorruptionPolicy, DecodeGateDecision,
    InsertDecision, InspectionAdmission, ReceiverDecodeContext, ReceiverState, RtpAccessUnit,
    SyntheticMarkerBoundary,
};
use crate::transport::rtc::session::facts::FrameValue as RecoveryFrameValue;
use crate::transport::rtc::stream::adapter_types::{
    TransportLossObservation, TransportObservation,
};
use crate::transport::rtc::stream::packet_types::{RtcVideoIngressKind, RtcVideoRepairMetadata};
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState,
    XbxEngineH264InspectionObservation, XbxEngineVideoRtxReinjectObservation,
};
use xbxengine_protocol::XbxEngineTransportStateDto;

use super::RtcVideoFrameSource;

const FIRST_FRAME_ACQUISITION_MAX_REQUEST_COUNT: u8 = 2;
const SAMPLE_LOSS_BURST_CLEAR_CLEAN_SAMPLE_COUNT: u8 = 6;
const IDLE_TIMEOUT_CONFIRMATION_GRACE_MIN_MS: u64 = 120;
const IDLE_TIMEOUT_CONFIRMATION_GRACE_MAX_MS: u64 = 220;
const THIN_STREAM_CONFIRMATION_GRACE_MIN_MS: u64 = 90;
const THIN_STREAM_CONFIRMATION_GRACE_MAX_MS: u64 = 180;
const FRAME_OOS_TRACK_CAPACITY: usize = 64;
const HEAD_MISSING_ACTIVITY_COOLDOWN_MS: f64 = 30_000.0;
const FRAME_PLAYOUT_BASE_TRACK_CAPACITY: usize = 96;

impl RtcVideoFrameSource {
    pub(crate) fn h264_continuation_verdict(
        inspection: &H264AccessUnitInspection,
        admission: InspectionAdmission,
        decode_ctx: &ReceiverDecodeContext,
    ) -> Option<String> {
        let continuation_ready = inspection.delta_continuation_ready()
            && inspection.committed_sps_present()
            && inspection.committed_pps_present();
        if matches!(admission, InspectionAdmission::Accept)
            && !inspection.bootstrap_ready
            && continuation_ready
            && !decode_ctx.hard_gap_blocks_delta()
        {
            return Some("receiverLocalContinuation".to_string());
        }
        let bootstrap_blocks = inspection_bootstrap_blocks_delta_continuation(inspection)
            || receiver_state_blocks_delta_continuation(decode_ctx);
        if continuation_ready && !bootstrap_blocks {
            return Some("continuationReady".to_string());
        }
        None
    }

    pub(crate) fn maybe_ack_clean_anchor_commit_from_runtime_stats(&mut self) {
        self.maybe_align_recovery_ledger_transport_epoch();
        let Some(committed_epoch) = self
            .runtime_stats
            .read(|stats| {
                (stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch))
                    .then_some(stats.transport_recovery_epoch)
            })
            .flatten()
        else {
            return;
        };
        if self.last_consumed_clean_anchor_epoch == Some(committed_epoch) {
            return;
        }
        self.last_consumed_clean_anchor_epoch = Some(committed_epoch);
        let (clean_anchor_rtp, clean_anchor_at_ms, display_rtp, display_at_ms) = self
            .runtime_stats
            .read(|stats| {
                (
                    stats
                        .recovery_displayed_idr_rtp
                        .or(stats.recovery_pending_displayed_idr_rtp)
                        .or(stats.latest_video_decode_ok_rtp_timestamp),
                    stats.video_anchor_clean_observed_at_ms,
                    stats.recovery_displayed_idr_rtp,
                    stats.recovery_displayed_idr_at_ms,
                )
            })
            .unwrap_or((None, None, None, None));
        let observed_at_ms = clean_anchor_at_ms.unwrap_or_else(now_ms_f64);
        self.runtime_stats.update(|stats| {
            if stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe")
                && stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch)
            {
                stats.video_decoder_recovery_state = Some("nominal".to_string());
                stats.video_decoder_recovery_event = Some("clean-anchor-committed".to_string());
                stats.video_decoder_recovery_detail = None;
                stats.video_decoder_recovery_status = None;
                stats.video_decoder_recovery_state_changed_at_ms = Some(observed_at_ms);
            }
        });
        let stats_snapshot = self
            .runtime_stats
            .read(|stats| stats.clone())
            .unwrap_or_default();
        if let Some(rtp) = clean_anchor_rtp {
            self.trace_ledger
                .recovery_ledger_mut()
                .note_usable_idr_packet_accepted(
                    rtp,
                    clean_anchor_at_ms.unwrap_or_else(now_ms_f64),
                );
        }
        if let Some(at_ms) = display_at_ms {
            self.trace_ledger
                .recovery_ledger_mut()
                .note_display_stable(display_rtp, at_ms);
        }
        self.trace_ledger
            .note_clean_anchor_committed(clean_anchor_rtp);
        self.set_is_blocking_non_keyframe_admission(false);
        self.receive_core_mut()
            .receive_engine
            .clear_recovery_state_after_decoded_anchor();
        self.trace_ledger
            .recovery_ledger_mut()
            .apply_decoder_facts_from_stats(&stats_snapshot, observed_at_ms);
        self.sync_recovery_ledger_to_stats();
        self.record_video_timeline_observation(
            "clean-anchor-committed",
            None,
            clean_anchor_rtp,
            observed_at_ms,
        );
    }

    pub(crate) async fn handle_drop_and_request_keyframe_action(
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
            self.record_video_timeline_observation(
                "frame-drop-loss-no-missing-seq-await-recovery",
                None,
                Some(sample_rtp_timestamp),
                now_ms,
            );
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

    pub(crate) fn maybe_emit_jitter_early_boundary(&mut self) {
        if !self.jitter_early_emit_enabled {
            return;
        }
        let Some(boundary) = self.pending_marker_boundary else {
            return;
        };
        if boundary.observed_at.elapsed() < self.jitter_early_emit_wait {
            return;
        }
        let media_ssrc = self.current_media_ssrc.unwrap_or_default();
        self.receive_core_mut()
            .receive_engine
            .frame_assembler
            .push_synthetic_aud_boundary(
                SyntheticMarkerBoundary {
                    sequence: boundary.sequence,
                    rtp_timestamp: boundary.rtp_timestamp,
                    media_payload_type: boundary.media_payload_type,
                },
                media_ssrc,
            );
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

    pub(crate) fn mark_frame_head_missing_signal(&mut self, rtp_timestamp: u32) {
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

    pub(crate) fn frame_seen_head_missing(&self, rtp_timestamp: u32) -> bool {
        self.frame_head_missing_flags
            .iter()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
            .is_some_and(|(_, flag)| *flag)
    }

    pub(crate) fn head_missing_recently_active(&self, now_ms: f64) -> bool {
        self.recent_head_missing_active_until_ms
            .is_some_and(|until_ms| now_ms <= until_ms)
    }

    pub(crate) fn record_frame_drop_attribution(&mut self, rtp_timestamp: u32, dropped: u16) {
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

    pub(crate) fn attributed_drop_count_for_frame(&self, rtp_timestamp: u32, fallback: u16) -> u16 {
        self.frame_drop_buckets
            .iter()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
            .map(|(_, count)| *count)
            .unwrap_or(fallback)
    }

    pub(crate) fn remember_frame_playout_base_candidate(
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

    pub(crate) fn take_frame_first_packet_arrived_at(
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

    pub(crate) fn record_frame_first_packet_sequence(&mut self, rtp_timestamp: u32, sequence: u16) {
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

    pub(crate) fn take_frame_first_packet_sequence(&mut self, rtp_timestamp: u32) -> Option<u16> {
        let index = self
            .frame_first_packet_sequences
            .iter()
            .position(|(timestamp, _)| *timestamp == rtp_timestamp)?;
        self.frame_first_packet_sequences
            .remove(index)
            .map(|(_, sequence)| sequence)
    }

    pub(crate) fn response_oos_depth_p75(&self) -> Option<u16> {
        if self.recent_oos_depths.is_empty() {
            return None;
        }
        let mut samples: Vec<u16> = self.recent_oos_depths.iter().copied().collect();
        samples.sort_unstable();
        let p75_index = ((samples.len().saturating_sub(1) * 3) / 4).min(samples.len() - 1);
        Some(samples[p75_index])
    }

    pub(crate) fn build_ingress_materialization_fallback_budget(
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

    pub(crate) fn nack_maintenance_timeout(
        &self,
        base_timeout: std::time::Duration,
    ) -> std::time::Duration {
        let elapsed = self.last_nack_maintenance_tick_at.elapsed();
        let until_tick = if elapsed >= self.nack_maintenance_tick_interval {
            std::time::Duration::ZERO
        } else {
            self.nack_maintenance_tick_interval - elapsed
        };
        base_timeout.min(until_tick)
    }

    pub(crate) fn should_run_nack_maintenance_tick(&self) -> bool {
        self.last_nack_maintenance_tick_at.elapsed() >= self.nack_maintenance_tick_interval
    }

    pub(crate) fn maybe_retry_waiting_recovery_keyframe(&mut self, now_ms: f64) {
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
        let retry_interval_ms = self
            .runtime_stats
            .read(|stats| {
                crate::transport::rtc::recovery::contract::recovery_keyframe_retry_interval_ms_for_stats(
                    stats, now_ms,
                )
            })
            .unwrap_or(self.recovery_keyframe_retry_interval_ms);
        self.next_recovery_keyframe_retry_at_ms = Some(now_ms + retry_interval_ms);
        crate::xbx_log_warn!(
            "[RtcVideoFrameSource] recovery keyframe wait timeout retry count={} waiting_since_ms={:?}",
            self.recovery_keyframe_retry_count,
            self.waiting_recovery_keyframe_since_ms
        );
    }

    pub(crate) fn clear_pending_timeout_confirmations(&mut self) {
        self.pending_idle_timeout_since = None;
        self.pending_thin_stream_since = None;
    }

    pub(crate) fn first_frame_acquired(stats: &crate::XbxEngineMediaRuntimeStats) -> bool {
        if crate::transport::rtc::recovery::contract::has_current_clean_anchor_from_stats(stats) {
            return true;
        }
        stats
            .latest_video_decode_ok_time_ms
            .is_some_and(|decode_ok_ms| {
                Self::progress_time_fresh_for_active_recovery_episode(stats, decode_ok_ms)
            })
            || Self::receiver_local_first_frame_acquired_from_engine_stats(stats)
    }

    fn progress_time_fresh_for_active_recovery_episode(
        stats: &crate::XbxEngineMediaRuntimeStats,
        progress_ms: f64,
    ) -> bool {
        if !stats.transport_recovery_episode_active {
            return true;
        }
        stats
            .transport_recovery_episode_opened_at_ms
            .is_none_or(|opened_at_ms| progress_ms >= opened_at_ms)
    }

    fn receiver_local_first_frame_acquired_from_engine_stats(
        stats: &crate::XbxEngineMediaRuntimeStats,
    ) -> bool {
        stats
            .latest_h264_inspection_observation
            .as_ref()
            .is_some_and(|obs| obs.committed_sps_present && obs.committed_pps_present)
    }

    fn receiver_local_first_frame_acquired(&self) -> bool {
        let engine = &self.receive_core().receive_engine;
        engine.bootstrap.committed_sps_present()
            && engine.bootstrap.committed_pps_present()
            && engine.frame_assembler.assembled_count() > 0
    }

    fn receiver_decode_context(&self) -> ReceiverDecodeContext {
        let engine = &self.receive_core().receive_engine;
        let now_ms = now_ms_f64();
        let decoder_reference_synced = self
            .runtime_stats
            .read(|stats| {
                crate::transport::rtc::recovery::contract::decoder_reference_synced_from_stats(
                    stats, now_ms,
                )
            })
            .unwrap_or(false);
        ReceiverDecodeContext {
            receiver_state: self.receiver_local_state(),
            has_active_gap: engine.has_active_gap(),
            nack_exhausted: engine.nack_requester.nack_escalation_pending()
                || engine.nack_requester.has_exhausted_gaps(),
            first_frame_acquired: self.receiver_local_first_frame_acquired(),
            decoder_reference_synced,
        }
    }

    pub(crate) fn first_frame_acquisition_runtime_context(
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

    pub(crate) fn should_request_first_frame_acquisition_keyframe(
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

    pub(crate) fn should_request_first_frame_acquisition_followup_keyframe(
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

    /// 消费 session / decode 写入的 receive keyframe 提示。
    /// decoder reset 后需要进入硬恢复窗，阻止后续 delta continuation 继续喂给刚重置的后端。
    fn maybe_consume_recovery_receive_keyframe_hint(&mut self) {
        const HINT_TTL_MS: f64 = 2_000.0;
        let now_ms = now_ms_f64();
        let hint_at_ms = self
            .runtime_stats
            .read(|stats| stats.recovery_receive_keyframe_hint_at_ms)
            .flatten();
        let Some(hint_at_ms) = hint_at_ms else {
            return;
        };
        if (now_ms - hint_at_ms).max(0.0) > HINT_TTL_MS {
            self.runtime_stats
                .update(|stats| stats.recovery_receive_keyframe_hint_at_ms = None);
            return;
        }
        self.runtime_stats.update(|stats| {
            stats.recovery_receive_keyframe_hint_at_ms = None;
            stats.recovery_decoder_reference_synced_at_ms = None;
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
        });
        self.trace_ledger
            .recovery_ledger_mut()
            .note_decoder_waiting_keyframe();
        self.sync_recovery_ledger_to_stats();
        self.request_recovery_keyframe_from_source("recovery-receive-keyframe-hint", None, now_ms);
    }

    pub(crate) fn maybe_request_first_frame_acquisition_keyframe(
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
        let event_name = match request_kind {
            FirstFrameAcquisitionRequestKind::Initial => {
                "first-frame-acquisition-keyframe-requested"
            }
            FirstFrameAcquisitionRequestKind::Followup => {
                "first-frame-acquisition-keyframe-followup-requested"
            }
        };
        self.record_video_timeline_observation(event_name, None, frame_rtp_timestamp, now_ms);
        let source = match request_kind {
            FirstFrameAcquisitionRequestKind::Initial => "first-frame-acquisition",
            FirstFrameAcquisitionRequestKind::Followup => "first-frame-acquisition-followup",
        };
        self.request_receiver_local_keyframe(source, frame_rtp_timestamp, now_ms, false);
    }

    pub(crate) fn request_recovery_keyframe_from_source(
        &mut self,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) {
        self.request_receiver_local_keyframe(source_event, frame_rtp_timestamp, now_ms, false);
    }

    pub(crate) fn request_recovery_keyframe_soft_from_source(
        &mut self,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) {
        self.request_receiver_local_keyframe(source_event, frame_rtp_timestamp, now_ms, true);
    }

    pub(crate) fn enter_recovery_wait_from_source(
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
                    true,
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
        self.record_video_timeline_observation(source_event, None, frame_rtp_timestamp, now_ms);
        self.record_anchor_candidate_ledger(
            frame_rtp_timestamp,
            source_event,
            candidate_state,
            failure_reason,
            now_ms,
        );
        self.request_receiver_local_keyframe(source_event, frame_rtp_timestamp, now_ms, false);
    }

    pub(crate) fn maybe_seed_h264_bootstrap_from_remote_answer(&mut self) {
        if self
            .receive_core_mut()
            .receive_engine
            .bootstrap
            .committed_sps_present()
            && self
                .receive_core_mut()
                .receive_engine
                .bootstrap
                .committed_pps_present()
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
            .receive_core_mut()
            .receive_engine
            .bootstrap
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

    pub(crate) fn should_trigger_thin_stream_stall(&self, now: std::time::Instant) -> bool {
        self.receive_core()
            .receive_engine
            .frame_assembler
            .should_trigger_thin_stream_stall(
                now,
                self.assembly_stall_timeout,
                self.thin_stream_packet_threshold,
            )
    }

    /// receiver-local 恢复阶段不把 idle/thin-stream 超时升格为 transport observation。
    pub(crate) fn should_keep_timeout_recovery_receiver_local(&self) -> bool {
        matches!(
            self.receiver_local_state(),
            ReceiverState::WaitingKeyframe | ReceiverState::Repairing | ReceiverState::Priming
        )
    }

    /// repair / bootstrap / wait-keyframe 阶段不把 loss/idle 信号泄漏到 transport observation 通道。
    pub(crate) fn should_suppress_receiver_local_transport_observation(
        &self,
        observation: TransportObservation,
    ) -> bool {
        if !matches!(
            observation,
            TransportObservation::StreamIdleTimeout
                | TransportObservation::StreamThinStall
                | TransportObservation::Loss(_)
                | TransportObservation::NackRecoveredLate
        ) {
            return false;
        }
        if self.should_keep_timeout_recovery_receiver_local() {
            return true;
        }
        self.runtime_stats
            .read(|stats| !Self::first_frame_acquired(stats))
            .unwrap_or(true)
    }

    pub(crate) fn thin_stream_confirmation_grace(&self) -> std::time::Duration {
        self.assembly_stall_timeout.div_f32(3.0).clamp(
            std::time::Duration::from_millis(THIN_STREAM_CONFIRMATION_GRACE_MIN_MS),
            std::time::Duration::from_millis(THIN_STREAM_CONFIRMATION_GRACE_MAX_MS),
        )
    }

    pub(crate) fn idle_timeout_confirmation_grace(
        &self,
        idle_timeout: std::time::Duration,
    ) -> std::time::Duration {
        idle_timeout.div_f32(2.0).clamp(
            std::time::Duration::from_millis(IDLE_TIMEOUT_CONFIRMATION_GRACE_MIN_MS),
            std::time::Duration::from_millis(IDLE_TIMEOUT_CONFIRMATION_GRACE_MAX_MS),
        )
    }

    pub(crate) fn should_confirm_transient_timeout_signal(
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

    pub(crate) fn should_emit_confirmed_idle_timeout(
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

    pub(crate) fn should_emit_confirmed_thin_stream_stall(
        &mut self,
        now: std::time::Instant,
    ) -> bool {
        let confirmation_grace = self.thin_stream_confirmation_grace();
        Self::should_confirm_transient_timeout_signal(
            &mut self.pending_thin_stream_since,
            now,
            confirmation_grace,
        )
    }

    pub(crate) fn should_prioritize_reinject_drain(&self) -> bool {
        self.runtime_stats
            .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
            .flatten()
            .is_some_and(|observation| {
                observation.stage == "queued" && observation.pending_queue_len > 0
            })
    }

    pub(crate) fn reinject_observation_for_ingress(
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

    pub(crate) fn build_reinject_observation(
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

    pub(crate) fn record_reinject_stage(
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

    pub(crate) async fn evaluate_decode_gate(
        &mut self,
        sample: RtpAccessUnit,
    ) -> DecodeGateDecision {
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
        self.maybe_consume_recovery_receive_keyframe_hint();
        self.last_packet_time = std::time::Instant::now();
        let mut payload = sample.payload;
        self.maybe_request_first_frame_acquisition_keyframe(
            Some(sample.packet_timestamp),
            FirstFrameAcquisitionRequestKind::Initial,
        );
        self.maybe_seed_h264_bootstrap_from_remote_answer();
        let mut inspection = match self
            .receive_core_mut()
            .receive_engine
            .bootstrap
            .inspect_access_unit(&payload)
        {
            Ok(inspection) => inspection,
            Err(error) => {
                let now_ms = now_ms_f64();
                crate::xbx_log_error!("[RtcVideoFrameSource] h264 inspection failed: {error}");
                // 检查错误：保守处理为 disposable
                self.trace_ledger.observe_frame(
                    sample.packet_timestamp,
                    now_ms,
                    None,
                    "disposable",
                );
                self.trace_ledger.mark_frame_closed(
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
                return DecodeGateDecision::Continue;
            }
        };
        self.runtime_stats.update(|stats| {
            stats.recovery_codec_bootstrap_salvage_applied = None;
            stats.recovery_codec_bootstrap_salvage_failed_reason = None;
        });
        if let Some(salvaged) = self
            .receive_core_mut()
            .receive_engine
            .bootstrap
            .try_prepend_committed_parameter_sets(&inspection, &payload)
        {
            match self
                .receive_core_mut()
                .receive_engine
                .bootstrap
                .inspect_access_unit(&salvaged)
            {
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
        if inspection.is_idr {
            self.note_ingress_waiting_idr_inspection();
            self.receive_core_mut()
                .receive_engine
                .keyframe_requester
                .on_idr_received();
            self.receive_core_mut()
                .receive_engine
                .nack_requester
                .clear_keyframe_escalation();
            if inspection.bootstrap_ready
                || (inspection.slice_headers_valid
                    && inspection.committed_sps_present()
                    && inspection.committed_pps_present())
            {
                self.set_is_blocking_non_keyframe_admission(false);
            }
        }
        let inspection_now_ms = now_ms_f64();
        let decode_ctx = self.receiver_decode_context();
        let effective_rtt_ms = self
            .runtime_stats
            .read(|stats| stats.recovery_effective_rtt_ms.unwrap_or(200.0))
            .unwrap_or(200.0);
        let insert_ctx = self.build_insert_context(decode_ctx, inspection_now_ms, effective_rtt_ms);
        let (insert_decision, insert_reason): (InsertDecision, &str) =
            resolve_insert_decision_with_reason(
                &inspection,
                &insert_ctx,
                DecodeCorruptionPolicy::StandardWebRtc,
                sample
                    .prev_dropped_packets
                    .saturating_sub(sample.prev_padding_packets),
            );
        self.runtime_stats.update(|stats| {
            stats.latest_insert_decision = Some(insert_decision_label(insert_decision).to_string());
            stats.latest_insert_decision_reason = Some(insert_reason.to_string());
            stats.latest_packet_recovery_action_stage =
                Some(insert_ctx.action_stage.as_str().to_string());
            stats.insert_decode_bypass_aligned = None;
        });
        if inspection.is_idr {
            let idr_admitted_to_decode = matches!(insert_decision, InsertDecision::Emit)
                && (inspection.bootstrap_ready || insert_ctx.fresh_idr_admission);
            if idr_admitted_to_decode && inspection.bootstrap_ready {
                self.note_usable_idr_for_picture_recovery_terminal(
                    sample.packet_timestamp,
                    inspection_now_ms,
                );
            }
            if !idr_admitted_to_decode {
                self.trace_ledger.recovery_ledger_mut().note_idr_unusable();
                self.sync_recovery_ledger_to_stats();
                self.note_ingress_idr_not_admitted(insert_reason);
            }
        } else if !inspection.bootstrap_ready {
            if !self
                .trace_ledger
                .recovery_ledger()
                .has_established_usable_anchor
            {
                self.note_first_delta_for_recovery_ledger();
            } else if self
                .trace_ledger
                .recovery_ledger()
                .unresolved_keyframe_request_count
                > 0
                || self.trace_ledger.recovery_ledger().keyframe_required
            {
                self.note_non_idr_continuation_for_recovery_ledger();
            }
        }
        self.refresh_recovery_ledger_decoder_facts(inspection_now_ms);
        if matches!(insert_decision, InsertDecision::HoldRepair) {
            let now_ms = now_ms_f64();
            self.runtime_stats.record_h264_inspection_observation(
                XbxEngineH264InspectionObservation {
                    observation_id: u64::from(sample.packet_timestamp),
                    frame_rtp_timestamp: Some(sample.packet_timestamp),
                    nal_types: inspection.nal_type_labels(),
                    nal_count: inspection.nals.len() as u16,
                    vcl_nal_count: 0,
                    bootstrap_ready: inspection.bootstrap_ready,
                    bootstrap_reject_reason: inspection
                        .bootstrap_reject_reason
                        .map(|r| r.as_str().to_string()),
                    admission_accepted: false,
                    observed_at_ms: inspection_now_ms,
                    ..Default::default()
                },
            );
            self.trace_ledger
                .observe_frame(sample.packet_timestamp, now_ms, None, "unknown");
            self.trace_ledger.mark_frame_closed(
                sample.packet_timestamp,
                now_ms,
                None,
                "unknown",
                Some("insertGateHoldRepair"),
            );
            let reject_reason = inspection_bootstrap_reason(&inspection);
            if should_block_non_keyframe_admission(&decode_ctx) {
                self.set_is_blocking_non_keyframe_admission(true);
            }
            self.enter_recovery_wait_from_source(
                "insert-gate-hold-repair",
                Some(sample.packet_timestamp),
                XbxEngineAnchorCandidateState::AwaitingRecovery,
                None,
                reject_reason,
                RecoveryFrameValue::RecoveryAnchor,
                false,
                now_ms,
            );
            if self.should_request_first_frame_acquisition_followup_keyframe(&inspection) {
                self.maybe_request_first_frame_acquisition_keyframe(
                    Some(sample.packet_timestamp),
                    FirstFrameAcquisitionRequestKind::Followup,
                );
            }
            // HoldRepair 动作不变式：每拍必须尝试 receive-local keyframe（可观测 outcome）。
            let source_event = if insert_ctx.post_parameter_sets_change_strict {
                "insert-gate-post-ps-change-strict"
            } else if matches!(
                insert_ctx.reference_chain_state,
                crate::transport::rtc::recovery::contract::ReferenceChainState::NeedKeyframe
            ) {
                "insert-gate-need-keyframe"
            } else if insert_ctx.supply_break_continuation {
                "insert-gate-supply-break"
            } else if crate::transport::rtc::recovery::contract::gap_keyframe_only_mode_active(
                insert_ctx.gap_mode,
            ) {
                "insert-gate-keyframe-only"
            } else if self.sparse_idr_rhythm_for_receive(now_ms).active {
                "insert-gate-hold-repair-sparse"
            } else {
                "insert-gate-hold-repair"
            };
            self.request_recovery_keyframe_soft_from_source(
                source_event,
                Some(sample.packet_timestamp),
                now_ms,
            );
            return DecodeGateDecision::Continue;
        }
        if matches!(insert_decision, InsertDecision::DropCorrupt) {
            self.handle_drop_and_request_keyframe_action(
                sample.packet_timestamp,
                sample
                    .prev_dropped_packets
                    .saturating_sub(sample.prev_padding_packets),
                inspection.is_idr,
                if inspection.is_idr {
                    "keyframe"
                } else {
                    "delta"
                },
            )
            .await;
            return DecodeGateDecision::Continue;
        }
        let inspection_admission = insert_decision_to_inspection_admission(insert_decision);
        if matches!(insert_decision, InsertDecision::Emit)
            && decode_ctx.decoder_reference_synced
            && !decode_ctx.has_active_gap
        {
            self.set_is_blocking_non_keyframe_admission(false);
        }
        let should_block = should_block_non_keyframe_admission(&decode_ctx);
        if should_block != self.is_blocking_non_keyframe_admission() {
            self.set_is_blocking_non_keyframe_admission(should_block);
        }
        let is_blocking_non_keyframe_admission = self.is_blocking_non_keyframe_admission();
        let admission_accepted = matches!(inspection_admission, InspectionAdmission::Accept);
        if admission_accepted {
            // H264 tracker 属于 receive/packet 主链：完整 AU 准入后立即提交 SPS/PPS。
            // 下游 scheduler/decode 仍可维护自己的流参数，但 continuation 判定不能等 display/host。
            inspection.commit();
        }
        let response_detail =
            keyframe_episode_response_detail(&inspection, inspection_admission, Some(&decode_ctx));
        let continuation_verdict =
            Self::h264_continuation_verdict(&inspection, inspection_admission, &decode_ctx);
        // bootstrap_reject_reason 只描述当前 AU 是否具备自举条件，不代表 delta slice 不能继续承接。
        self.runtime_stats
            .record_h264_inspection_observation(XbxEngineH264InspectionObservation {
                observation_id: u64::from(sample.packet_timestamp),
                frame_rtp_timestamp: Some(sample.packet_timestamp),
                nal_types: inspection.nal_type_labels(),
                nal_count: inspection.nals.len() as u16,
                vcl_nal_count: inspection
                    .nals
                    .iter()
                    .filter(|nal| {
                        matches!(
                            nal.unit_type,
                            h264_reader::nal::UnitType::SliceLayerWithoutPartitioningIdr
                                | h264_reader::nal::UnitType::SliceLayerWithoutPartitioningNonIdr
                        )
                    })
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
            });
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
                self.trace_ledger
                    .observe_frame(sample.packet_timestamp, now_ms, None, "unknown");
                self.trace_ledger.mark_frame_closed(
                    sample.packet_timestamp,
                    now_ms,
                    None,
                    "unknown",
                    Some(reject_reason),
                );
                let rejection_source_event = "frame-inspection-rejected-await-anchor";
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
                return DecodeGateDecision::Continue;
            }
        }
        let is_keyframe = inspection.is_idr;
        let config_changed = inspection.config_changed;
        if media_dropped_packets > 0 {
            self.record_frame_drop_attribution(sample.packet_timestamp, media_dropped_packets);
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
        let recovery_action = recovery_keyframe_action_for_insert_decision(insert_decision);
        let next_is_blocking_non_keyframe_admission = match insert_decision {
            InsertDecision::Emit => false,
            InsertDecision::DropCorrupt => is_blocking_non_keyframe_admission,
            InsertDecision::HoldRepair => should_block_non_keyframe_admission(&decode_ctx),
        };

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
                return DecodeGateDecision::Continue;
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
                return DecodeGateDecision::Continue;
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
        self.trace_ledger.observe_frame(
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
        let first_packet_sequence = self.take_frame_first_packet_sequence(sample.packet_timestamp);
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
        let assembled_count = self
            .receive_core_mut()
            .receive_engine
            .frame_assembler
            .assembled_count();
        if assembled_count == 1 || assembled_count.is_power_of_two() {
            crate::xbx_log_info!(
                "[RtcVideoFrameSource] assembled frame count={} ts={} len={} keyframe={} bootstrap={}",
                assembled_count,
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
        // RFC：pre-decode 不用 decode/host 反馈做 sustaining/clean-anchor gate。
        let can_mark_complete_candidate = matches!(
            self.receiver_local_state(),
            ReceiverState::Receiving | ReceiverState::Repairing
        ) || current_frame_allows_sustaining_exit;
        if can_mark_complete_candidate {
            // 将媒体语义标签转换为恢复语义标签
            let recovery_importance = match media_type_label {
                "keyframe" => "anchor",
                "reference" => "supply",
                "delta" => "disposable",
                _ => "disposable",
            };
            self.trace_ledger.mark_frame_complete_candidate(
                sample.packet_timestamp,
                complete_candidate_now_ms,
                Some(is_keyframe),
                recovery_importance,
            );
            self.sync_recovery_ledger_to_stats();
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
        } else {
            self.record_video_timeline_observation(
                "frame-complete-candidate-decode-feedback-blocked",
                None,
                Some(sample.packet_timestamp),
                complete_candidate_now_ms,
            );
        }

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
        // clean-anchor stats 提交仅在 post-decode；ingress 不再带 pre-decode commit epoch。
        let clean_anchor_commit_recovery_epoch = None;
        let is_recovery_owner_frame = recovery_owner_rtp_timestamp == Some(sample.packet_timestamp);
        let eligible_recovery_owner_frame = frame_unrecoverable_reason.is_none()
            && is_recovery_owner_frame
            && current_frame_allows_sustaining_exit
            && inspection.is_idr;
        if eligible_recovery_owner_frame {
            frame_budget = frame_budget.promote_to_recovery_window(frame_value);
            if matches!(frame_recovery_disposition, FrameRecoveryDisposition::Steady) {
                frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
            }
        }

        return DecodeGateDecision::Emit(AssembledVideoFrame {
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
}
