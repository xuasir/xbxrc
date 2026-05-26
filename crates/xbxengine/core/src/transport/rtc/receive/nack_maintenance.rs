//! RFC：pre-decode NACK/RTCP 维护与 gap 观测；不承载 host/display 或全局 recovery owner 裁决。

use xbxengine_protocol::XbxEngineTargetTypeDto;

use rtc_rtcp::transport_feedbacks::transport_layer_nack::{
    nack_pairs_from_sequence_numbers, TransportLayerNack,
};
use rtc_shared::marshal::{Marshal, MarshalSize};

use crate::media::video::ingress::budget::{
    DynamicRepairValueTier, FrameBudgetContext, FrameBudgetLinkValue, FrameBudgetWindowSource,
};
use crate::media::video::types::FrameRecoveryDisposition;
use crate::transport::rtc::receive::nack_policy::{
    cloud_nack_rtt_margin_ms, cloud_startup_head_hole_deadline_at_ms, sample_loss_nack_policy,
    OOS_REPAIRABILITY_PENALTY,
};
use crate::transport::rtc::recovery::contract::{
    gap_keyframe_only_mode_active, resolve_gap_vs_keyframe_mode, GapVsKeyframeMode,
};
use crate::transport::rtc::recovery::policy::ScenarioPolicyResolver;
use crate::transport::rtc::recovery::runtime_state::resolve_runtime_recovery_profile;
use crate::transport::rtc::recovery::timing::{
    merge_nack_admission_deadline_with_dynamic_timeout, resolve_effective_rtt_ms,
    resolve_recovery_dynamic_timing,
};
use crate::transport::rtc::stream::nack_contract::{
    NackBatch, PacketRecoveryDisposition, ResolvedNack, SkippedNackBatch,
};
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState,
    XbxEngineVideoNackObservation,
};

use crate::media::video::types::FrameValue;
use crate::transport::rtc::receive::ingress_state::{RecentRtpPacket, RtcVideoFrameSource};
use crate::transport::rtc::receive::now_ms_f64;
use crate::transport::rtc::stream::adapter_types::TransportObservation;

const LOW_VALUE_NEAR_DEADLINE_GUARD_MS: f64 = 12.0;
const SUPPLY_NEAR_DEADLINE_GUARD_MS: f64 = 6.0;

/// transport 路径上仅有显式 keyframe 标记才写入 gap 的媒体证据 importance。
pub(super) fn gap_transport_evidence(frame_is_keyframe: Option<bool>) -> &'static str {
    if frame_is_keyframe == Some(true) {
        "anchor"
    } else {
        "unknown"
    }
}

impl RtcVideoFrameSource {
    pub(super) async fn maybe_run_receiver_local_nack_maintenance(&mut self, now_ms: f64) {
        use crate::media::video::ingress::budget::FrameBudgetWindowSource;
        use crate::media::video::types::FrameValue;

        let now = std::time::Instant::now();
        let effective_rtt_ms = self
            .runtime_stats
            .read(|stats| {
                resolve_effective_rtt_ms(
                    stats,
                    ScenarioPolicyResolver::resolve_kind(
                        stats.session_target_type.as_ref(),
                        stats.transport_path.as_deref(),
                    ),
                )
            })
            .unwrap_or(100.0);
        let poll = self
            .receive_core_mut()
            .receive_engine
            .poll_nack_maintenance(now, effective_rtt_ms);
        if !poll.sequences.is_empty() {
            let max_retry = poll.retry_counts.iter().copied().max().unwrap_or(0);
            let frame_value = FrameValue::new(false, false, 12 * 1024);
            let budget_context = FrameBudgetContext::for_transport(
                frame_value,
                self.is_blocking_non_keyframe_admission(),
                None,
                None,
                None,
                false,
                FrameBudgetWindowSource::Transport,
            );
            let batch = NackBatch {
                sequences: poll.sequences,
                retry_count: max_retry,
                source: "receiverLocal",
                frame_rtp_timestamp: None,
                frame_is_keyframe: None,
                frame_importance: "transport",
                deadline_at_ms: None,
                estimated_recovery_arrival_ms: None,
                frame_playout_deadline_at_ms: None,
                nack_disposition: PacketRecoveryDisposition::Attempted,
                frame_unrecoverable_reason: None,
                budget_context,
            };
            let _ = self.send_nack_batch("sent", &batch, now_ms).await;
        }
        if poll.keyframe_escalation_due {
            self.request_receiver_local_keyframe(
                "receiver-local-nack-escalation",
                None,
                now_ms,
                false,
            );
            self.receive_core_mut()
                .receive_engine
                .nack_requester
                .on_keyframe_escalation_sent();
        }
        if self.is_blocking_non_keyframe_admission() {
            let capability = self.receive_core().transport_capability.clone();
            let _ = self
                .receive_core_mut()
                .receive_engine
                .keyframe_requester
                .request_if_due(capability.as_ref(), true);
        }
    }

    pub(super) async fn maybe_run_nack_maintenance(&mut self) {
        let now_ms = now_ms_f64();
        let now = std::time::Instant::now();
        // 只有真正到了 tick 间隔才更新时间戳，避免包到达路径频繁调用时持续推迟下次 tick。
        if self.should_run_nack_maintenance_tick() {
            self.last_nack_maintenance_tick_at = now;

            // 定期清理帧边界追踪状态，将已确认完成的帧从 active 移到 completed
            if let Ok(mut tracker) = self.frame_boundary.lock() {
                tracker.maybe_finalize_frames(now);
            }
        }
        self.maybe_retry_waiting_recovery_keyframe(now_ms);
        self.maybe_run_receiver_local_nack_maintenance(now_ms).await;
        self.runtime_stats.set_video_pending_missing_packets(
            self.receive_core().receive_engine.pending_nack_count(),
        );
    }

    pub(super) fn record_receiver_local_nack_recovered(
        &mut self,
        sequence: u16,
        now_ms: f64,
        was_late: bool,
    ) {
        use crate::media::video::ingress::budget::FrameBudgetContext;

        let frame_value = FrameValue::new(false, false, 12 * 1024);
        let budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.is_blocking_non_keyframe_admission(),
            None,
            None,
            None,
            false,
            FrameBudgetWindowSource::Transport,
        );
        self.record_nack_recovered(
            ResolvedNack {
                sequence,
                recovery_time_ms: 0.0,
                retry_count: 0,
                was_late,
                source: "receiverLocal",
                frame_rtp_timestamp: None,
                frame_is_keyframe: None,
                frame_importance: "transport",
                deadline_at_ms: None,
                estimated_recovery_arrival_ms: None,
                frame_playout_deadline_at_ms: None,
                nack_disposition: PacketRecoveryDisposition::Attempted,
                frame_unrecoverable_reason: None,
                budget_context,
            },
            now_ms,
        );
    }

    pub(super) async fn send_nack_batch(
        &mut self,
        action: &str,
        batch: &NackBatch,
        now_ms: f64,
    ) -> Result<(), String> {
        if batch.sequences.is_empty() {
            return Ok(());
        }

        let media_ssrc = self.current_media_ssrc.ok_or_else(|| {
            crate::xbx_log_warn!(
                "[RtcVideoFrameSource] skip nack send action={} because media ssrc is unavailable",
                action
            );
            "media ssrc unavailable".to_string()
        })?;

        let nack = TransportLayerNack {
            sender_ssrc: self.local_rtcp_sender_ssrc,
            media_ssrc,
            nacks: nack_pairs_from_sequence_numbers(&batch.sequences),
        };

        let mut buf = vec![0u8; nack.marshal_size()];
        nack.marshal_to(&mut buf).map_err(|_| {
            crate::xbx_log_warn!(
                "[RtcVideoFrameSource] nack serialize failed action={}",
                action
            );
            "nack serialize failed".to_string()
        })?;

        use crate::transport::rtc::capability::TransportCapabilityError;
        self.receive_core_mut()
            .transport_capability
            .send_nack_rtcp(&buf)
            .map_err(|error| {
                let detail = match error {
                    TransportCapabilityError::SendFailed { detail } => detail,
                    TransportCapabilityError::FeedbackUnavailable { detail }
                    | TransportCapabilityError::TransportNotReady { detail } => detail,
                };
                crate::xbx_log_warn!(
                    "[RtcVideoFrameSource] nack send failed action={} error={}",
                    action,
                    detail
                );
                detail
            })?;

        // 只在真正发送成功后记录统计
        self.runtime_stats.record_nack_sent(
            batch.sequences.len(),
            self.receive_core().receive_engine.pending_nack_count(),
        );
        self.record_nack_observation(action, batch, now_ms);

        Ok(())
    }

    fn record_nack_observation(&mut self, action: &str, batch: &NackBatch, now_ms: f64) {
        let sequences = &batch.sequences;
        let Some(first_sequence) = sequences.first().copied() else {
            return;
        };
        let Some(last_sequence) = sequences.last().copied() else {
            return;
        };
        self.nack_observation_id = self.nack_observation_id.saturating_add(1);
        self.runtime_stats
            .record_latest_video_nack_observation(XbxEngineVideoNackObservation {
                observation_id: self.nack_observation_id,
                action: action.to_string(),
                source: batch.source.to_string(),
                first_sequence,
                last_sequence,
                packet_count: sequences.len().min(u16::MAX as usize) as u16,
                retry_count: batch.retry_count,
                frame_rtp_timestamp: batch.frame_rtp_timestamp,
                frame_is_keyframe: batch.frame_is_keyframe,
                frame_importance: Some(batch.frame_importance.to_string()),
                deadline_at_ms: batch.deadline_at_ms,
                estimated_recovery_arrival_ms: batch.estimated_recovery_arrival_ms,
                nack_disposition: Some(batch.nack_disposition.as_str().to_string()),
                frame_playout_deadline_at_ms: batch.frame_playout_deadline_at_ms,
                frame_unrecoverable_reason: batch
                    .frame_unrecoverable_reason
                    .map(|reason| reason.to_string()),
                frame_budget: None,
                observed_at_ms: now_ms,
            });
        self.record_video_timeline_observation(
            "nack-observation",
            Some(first_sequence),
            batch.frame_rtp_timestamp,
            now_ms,
        );
    }

    pub(super) fn record_nack_recovered(&mut self, resolved: ResolvedNack, now_ms: f64) {
        self.record_frame_recovery_from_nack(
            resolved.frame_rtp_timestamp,
            resolved.frame_importance,
            resolved.nack_disposition,
            resolved.frame_playout_deadline_at_ms,
            resolved.frame_unrecoverable_reason,
            resolved.budget_context,
            now_ms,
        );
        self.nack_recovery_ewma_ms =
            (self.nack_recovery_ewma_ms * 0.8) + (resolved.recovery_time_ms * 0.2);
        self.nack_late_ewma = if resolved.was_late {
            (self.nack_late_ewma * 0.8) + 0.2
        } else {
            self.nack_late_ewma * 0.8
        };
        self.nack_observation_id = self.nack_observation_id.saturating_add(1);
        self.runtime_stats.record_nack_recovered(
            resolved.was_late,
            resolved.recovery_time_ms,
            self.receive_core().receive_engine.pending_nack_count(),
            XbxEngineVideoNackObservation {
                observation_id: self.nack_observation_id,
                action: if resolved.was_late {
                    "recoveredLate".to_string()
                } else {
                    "recovered".to_string()
                },
                source: resolved.source.to_string(),
                first_sequence: resolved.sequence,
                last_sequence: resolved.sequence,
                packet_count: 1,
                retry_count: resolved.retry_count,
                frame_rtp_timestamp: resolved.frame_rtp_timestamp,
                frame_is_keyframe: resolved.frame_is_keyframe,
                frame_importance: Some(resolved.frame_importance.to_string()),
                deadline_at_ms: resolved.deadline_at_ms,
                estimated_recovery_arrival_ms: resolved.estimated_recovery_arrival_ms,
                nack_disposition: Some(resolved.nack_disposition.as_str().to_string()),
                frame_playout_deadline_at_ms: resolved.frame_playout_deadline_at_ms,
                frame_unrecoverable_reason: resolved
                    .frame_unrecoverable_reason
                    .map(|reason| reason.to_string()),
                frame_budget: None,
                observed_at_ms: now_ms,
            },
        );
        if resolved.was_late {
            self.queue_transport_observation(TransportObservation::NackRecoveredLate);
        }
    }

    pub(super) fn record_packet_gap_observation(
        &mut self,
        missing_sequences: &[u16],
        inserted_count: u16,
        now_ms: f64,
        source: &str,
        frame_rtp_timestamp: Option<u32>,
        frame_packet_count: Option<u16>,
        frame_missing_count: Option<u16>,
        frame_is_keyframe: Option<bool>,
        frame_importance: Option<&str>,
    ) {
        let Some(first_sequence) = missing_sequences.first().copied() else {
            return;
        };
        let Some(last_sequence) = missing_sequences.last().copied() else {
            return;
        };
        self.packet_gap_observation_id = self.packet_gap_observation_id.saturating_add(1);
        self.runtime_stats.record_latest_video_packet_gap(
            crate::XbxEngineVideoPacketGapObservation {
                observation_id: self.packet_gap_observation_id,
                expected_sequence: first_sequence,
                received_sequence: last_sequence.wrapping_add(1),
                missing_count: inserted_count,
                source: source.to_string(),
                frame_rtp_timestamp,
                frame_packet_count,
                frame_missing_count,
                frame_is_keyframe,
                frame_importance: frame_importance.map(|value| value.to_string()),
                observed_at_ms: now_ms,
            },
            last_sequence,
        );
    }

    fn should_skip_low_value_sample_loss_nack(
        &self,
        frame_is_keyframe: bool,
        frame_importance: &'static str,
        now_ms: f64,
    ) -> bool {
        if frame_is_keyframe || matches!(frame_importance, "anchor" | "supply" | "reference") {
            return false;
        }
        self.sample_loss_burst_count >= 3
            || self.oos_recently_active(now_ms)
            || self.head_missing_recently_active(now_ms)
    }

    pub(super) async fn observe_sample_loss_and_nack(
        &mut self,
        sample_rtp_timestamp: u32,
        media_dropped_packets: u16,
        frame_is_keyframe: bool,
        frame_importance: &'static str,
    ) -> bool {
        let now_ms = now_ms_f64();
        if self.should_skip_low_value_sample_loss_nack(frame_is_keyframe, frame_importance, now_ms)
        {
            return false;
        }
        let attributed_dropped_packets =
            self.attributed_drop_count_for_frame(sample_rtp_timestamp, media_dropped_packets);
        let mut missing_sequences = self
            .collect_missing_sequences_for_sample(sample_rtp_timestamp, attributed_dropped_packets);
        let mut used_recent_fallback = false;
        if missing_sequences.is_empty() {
            used_recent_fallback = true;
            missing_sequences = self.collect_recent_missing_sequences(attributed_dropped_packets);
        }
        if missing_sequences.is_empty() {
            return false;
        }
        let budget_importance = if frame_is_keyframe {
            "anchor"
        } else {
            frame_importance
        };
        let evidence = gap_transport_evidence(Some(frame_is_keyframe));
        self.trace_ledger.observe_gap(
            &missing_sequences,
            now_ms,
            Some(sample_rtp_timestamp),
            budget_importance,
            evidence,
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-observed-sample-loss",
                Some(sequence),
                Some(sample_rtp_timestamp),
                now_ms,
            );
        }
        self.trace_ledger.mark_gap_nack_candidate(
            &missing_sequences,
            now_ms,
            Some(sample_rtp_timestamp),
            frame_importance,
            evidence,
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-nack-candidate",
                Some(sequence),
                Some(sample_rtp_timestamp),
                now_ms,
            );
        }
        self.receive_core_mut()
            .receive_engine
            .nack_requester
            .register_gaps(missing_sequences.iter().copied());
        let pending_before = self.receive_core_mut().receive_engine.pending_nack_count();
        use crate::media::video::ingress::budget::FrameBudgetWindowSource;
        let frame_value = FrameValue::new(frame_is_keyframe, false, 12 * 1024);
        let budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.is_blocking_non_keyframe_admission(),
            None,
            None,
            None,
            false,
            FrameBudgetWindowSource::Transport,
        );
        let cloud_mode = self.is_cloud_transport_profile();
        let cloud_startup_mode = self.is_cloud_startup_transport_profile();
        let cloud_rtt_ms = Some(self.cloud_nack_rtt_ms().max(0.0));
        let repairability = sample_loss_repairability(
            frame_is_keyframe,
            frame_importance,
            self.oos_recently_active(now_ms),
        );
        let nack_timeout_ms = self
            .runtime_stats
            .read(|stats| {
                let profile = resolve_runtime_recovery_profile(stats);
                resolve_recovery_dynamic_timing(stats, profile).nack_timeout_ms
            })
            .unwrap_or(120.0);
        let base_deadline_at_ms = now_ms + nack_timeout_ms.max(0.0);
        let policy = sample_loss_nack_policy(
            sample_rtp_timestamp,
            frame_is_keyframe,
            budget_context.clone(),
            base_deadline_at_ms,
            repairability,
            cloud_mode,
            cloud_startup_mode,
            cloud_rtt_ms,
        );
        let mut deadline_at_ms = merge_nack_admission_deadline_with_dynamic_timeout(
            now_ms,
            policy.deadline_at_ms.unwrap_or(base_deadline_at_ms),
            policy.frame_importance,
            nack_timeout_ms,
            policy.frame_playout_deadline_at_ms,
        );
        if cloud_mode {
            deadline_at_ms = cloud_startup_head_hole_deadline_at_ms(
                now_ms,
                deadline_at_ms,
                cloud_mode,
                cloud_startup_mode,
                cloud_rtt_ms,
                Some(nack_timeout_ms),
            );
        }
        let estimated_recovery_arrival_ms = if cloud_mode {
            now_ms + cloud_nack_rtt_margin_ms(cloud_startup_mode, cloud_rtt_ms).max(8.0)
        } else {
            let rtt = cloud_rtt_ms.unwrap_or(40.0).max(0.0);
            now_ms + (0.75 * rtt + 8.0).clamp(8.0, 40.0)
        };
        let repair_tier = classify_repair_value_tier(
            budget_context.clone(),
            frame_is_keyframe,
            cloud_startup_mode,
        );
        if repair_tier == FrameBudgetLinkValue::Disposable
            && should_skip_low_value_near_deadline(
                estimated_recovery_arrival_ms,
                Some(deadline_at_ms),
                LOW_VALUE_NEAR_DEADLINE_GUARD_MS,
            )
        {
            self.record_nack_skipped(
                &SkippedNackBatch {
                    sequences: missing_sequences.clone(),
                    source: policy.source,
                    frame_rtp_timestamp: Some(sample_rtp_timestamp),
                    frame_is_keyframe: Some(frame_is_keyframe),
                    frame_importance: policy.frame_importance,
                    deadline_at_ms: Some(deadline_at_ms),
                    estimated_recovery_arrival_ms: Some(estimated_recovery_arrival_ms),
                    frame_playout_deadline_at_ms: Some(deadline_at_ms),
                    nack_disposition: PacketRecoveryDisposition::SkippedLowValue,
                    frame_unrecoverable_reason: Some("estimatedArrivalNearDeadline"),
                    budget_context,
                },
                now_ms,
            );
            return false;
        }
        if !frame_is_keyframe
            && !matches!(policy.frame_importance, "anchor")
            && should_skip_non_anchor_near_deadline(
                estimated_recovery_arrival_ms,
                Some(deadline_at_ms),
                SUPPLY_NEAR_DEADLINE_GUARD_MS,
            )
        {
            self.record_nack_skipped(
                &SkippedNackBatch {
                    sequences: missing_sequences.clone(),
                    source: policy.source,
                    frame_rtp_timestamp: Some(sample_rtp_timestamp),
                    frame_is_keyframe: Some(frame_is_keyframe),
                    frame_importance: policy.frame_importance,
                    deadline_at_ms: Some(deadline_at_ms),
                    estimated_recovery_arrival_ms: Some(estimated_recovery_arrival_ms),
                    frame_playout_deadline_at_ms: Some(deadline_at_ms),
                    nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                    frame_unrecoverable_reason: Some("estimatedArrivalNearDeadlineSupplyRecovery"),
                    budget_context,
                },
                now_ms,
            );
            return false;
        }
        let gap_mode = self
            .runtime_stats
            .read(|stats| resolve_gap_vs_keyframe_mode(stats, now_ms, cloud_rtt_ms.unwrap_or(40.0)))
            .unwrap_or(GapVsKeyframeMode::RepairFirst);
        if gap_keyframe_only_mode_active(gap_mode) {
            self.request_receiver_local_keyframe(
                "gap-keyframe-only-mode",
                Some(sample_rtp_timestamp),
                now_ms,
                false,
            );
        }
        let batch = NackBatch {
            sequences: missing_sequences.clone(),
            retry_count: 0,
            source: if used_recent_fallback {
                "sampleLossFallback"
            } else {
                policy.source
            },
            frame_rtp_timestamp: Some(sample_rtp_timestamp),
            frame_is_keyframe: Some(frame_is_keyframe),
            frame_importance: policy.frame_importance,
            deadline_at_ms: Some(deadline_at_ms),
            estimated_recovery_arrival_ms: Some(estimated_recovery_arrival_ms),
            frame_playout_deadline_at_ms: policy
                .frame_playout_deadline_at_ms
                .or(Some(deadline_at_ms)),
            nack_disposition: policy.nack_disposition,
            frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
            budget_context,
        };
        if matches!(gap_mode, GapVsKeyframeMode::RepairFirst) {
            self.trace_ledger.mark_gap_repair_in_flight(
                &batch.sequences,
                now_ms,
                batch.frame_rtp_timestamp,
                batch.frame_importance,
                gap_transport_evidence(batch.frame_is_keyframe),
            );
            if let Some(sequence) = batch.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-repair-in-flight",
                    Some(sequence),
                    batch.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.record_anchor_candidate_ledger(
                batch.frame_rtp_timestamp,
                "gap-repair-in-flight",
                XbxEngineAnchorCandidateState::AwaitingRecovery,
                Some(XbxEngineAnchorCandidateFailureReason::LocalRepairPending),
                now_ms,
            );
        }
        let inserted_count = self
            .receive_core_mut()
            .receive_engine
            .pending_nack_count()
            .saturating_sub(pending_before)
            .min(u16::MAX as usize) as u16;
        if inserted_count > 0 {
            self.record_packet_gap_observation(
                &missing_sequences,
                inserted_count,
                now_ms,
                if used_recent_fallback {
                    "sampleLossFallback"
                } else {
                    "sampleLoss"
                },
                Some(sample_rtp_timestamp),
                Some((missing_sequences.len() + 1).min(u16::MAX as usize) as u16),
                Some(attributed_dropped_packets),
                Some(frame_is_keyframe),
                Some(frame_importance),
            );
        }
        let _ = self.send_nack_batch("sent", &batch, now_ms).await;
        true
    }

    fn record_nack_skipped(&mut self, skipped: &SkippedNackBatch, now_ms: f64) {
        self.record_frame_recovery_from_nack(
            skipped.frame_rtp_timestamp,
            skipped.frame_importance,
            skipped.nack_disposition,
            skipped.frame_playout_deadline_at_ms,
            skipped.frame_unrecoverable_reason,
            skipped.budget_context,
            now_ms,
        );
        self.record_nack_observation(
            "skipped",
            &NackBatch {
                sequences: skipped.sequences.clone(),
                retry_count: 0,
                source: skipped.source,
                frame_rtp_timestamp: skipped.frame_rtp_timestamp,
                frame_is_keyframe: skipped.frame_is_keyframe,
                frame_importance: skipped.frame_importance,
                deadline_at_ms: skipped.deadline_at_ms,
                estimated_recovery_arrival_ms: skipped.estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: skipped.frame_playout_deadline_at_ms,
                nack_disposition: skipped.nack_disposition,
                frame_unrecoverable_reason: skipped.frame_unrecoverable_reason,
                budget_context: skipped.budget_context,
            },
            now_ms,
        );
    }

    fn record_frame_recovery_from_nack(
        &mut self,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
        nack_disposition: PacketRecoveryDisposition,
        frame_playout_deadline_at_ms: Option<f64>,
        frame_unrecoverable_reason: Option<&'static str>,
        budget_context: FrameBudgetContext,
        observed_at_ms: f64,
    ) {
        let Some(frame_recovery_disposition) =
            self.resolve_frame_recovery_disposition_from_nack(frame_importance, nack_disposition)
        else {
            return;
        };
        self.record_frame_recovery_ledger(
            frame_rtp_timestamp,
            frame_playout_deadline_at_ms,
            frame_recovery_disposition,
            frame_unrecoverable_reason,
            budget_context,
            observed_at_ms,
        );
    }

    fn resolve_frame_recovery_disposition_from_nack(
        &self,
        frame_importance: &'static str,
        nack_disposition: PacketRecoveryDisposition,
    ) -> Option<FrameRecoveryDisposition> {
        if !self.is_cloud_transport_profile() {
            return None;
        }
        if !matches!(
            nack_disposition,
            PacketRecoveryDisposition::SkippedTooLate
                | PacketRecoveryDisposition::SkippedLowValue
                | PacketRecoveryDisposition::SkippedChainBroken
        ) {
            return None;
        }
        if frame_importance == "disposable" {
            return Some(FrameRecoveryDisposition::UnrecoverableLate);
        }
        if matches!(
            nack_disposition,
            PacketRecoveryDisposition::SkippedChainBroken
        ) {
            return Some(FrameRecoveryDisposition::UnrecoverableReferenceChain);
        }
        if matches!(frame_importance, "reference" | "anchor" | "keyframe",) {
            return Some(FrameRecoveryDisposition::UnrecoverableReferenceChain);
        }
        Some(FrameRecoveryDisposition::UnrecoverableSupplyMiss)
    }

    fn collect_recent_missing_sequences(&self, media_dropped_packets: u16) -> Vec<u16> {
        let mut missing = self
            .receive_core()
            .receive_engine
            .packet_buffer
            .all_missing();
        let desired = usize::from(media_dropped_packets.max(1))
            .saturating_mul(2)
            .max(4);
        let desired = if self.is_cloud_startup_transport_profile() {
            desired.max(16)
        } else if self.is_cloud_transport_profile() {
            desired.max(8)
        } else {
            desired
        };
        if missing.len() > desired {
            missing.truncate(desired);
        }
        missing
    }

    fn collect_missing_sequences_for_sample(
        &self,
        sample_rtp_timestamp: u32,
        media_dropped_packets: u16,
    ) -> Vec<u16> {
        let mut matching_packets = self
            .recent_rtp_packets
            .iter()
            .filter(|packet| packet.rtp_timestamp == sample_rtp_timestamp);
        let Some(first_packet) = matching_packets.next() else {
            return Vec::new();
        };
        let mut last_packet = *first_packet;
        for packet in matching_packets {
            last_packet = *packet;
        }

        let expand = media_dropped_packets.max(2).min(12);
        let start = first_packet.sequence.wrapping_sub(expand);
        let end_exclusive = last_packet.sequence.wrapping_add(expand.saturating_add(1));
        let mut missing = self
            .receive_core()
            .receive_engine
            .packet_buffer
            .missing_in_range(start, end_exclusive);
        let desired = usize::from(media_dropped_packets.max(1))
            .saturating_mul(3)
            .max(6);
        let desired = if self.is_cloud_startup_transport_profile() {
            desired.max(20)
        } else if self.is_cloud_transport_profile() {
            desired.max(12)
        } else {
            desired
        };
        if missing.len() > desired {
            missing.truncate(desired);
        }
        missing
    }

    pub(super) fn push_recent_rtp_packet(&mut self, sequence: u16, rtp_timestamp: u32) {
        if self.recent_rtp_packets.len() >= 512 {
            self.recent_rtp_packets.pop_front();
        }
        self.recent_rtp_packets.push_back(RecentRtpPacket {
            sequence,
            rtp_timestamp,
        });
    }

    fn is_cloud_transport_profile(&self) -> bool {
        self.runtime_stats
            .read(|stats| {
                matches!(
                    stats.session_target_type,
                    Some(XbxEngineTargetTypeDto::Cloud)
                ) || matches!(
                    stats.transport_policy_profile.as_deref(),
                    Some("cloudGaming")
                )
            })
            .unwrap_or(false)
    }

    pub(super) fn is_cloud_startup_transport_profile(&self) -> bool {
        self.runtime_stats
            .read(|stats| {
                let cloud_mode = matches!(
                    stats.session_target_type,
                    Some(XbxEngineTargetTypeDto::Cloud)
                ) || matches!(
                    stats.transport_policy_profile.as_deref(),
                    Some("cloudGaming")
                );
                cloud_mode
                    && (matches!(stats.session_phase.as_deref(), Some("startup"))
                        || matches!(
                            stats.direct_gaming_bitrate_band.as_deref(),
                            Some("startupLow")
                        ))
            })
            .unwrap_or(false)
    }

    pub(super) fn cloud_nack_rtt_ms(&self) -> f64 {
        self.runtime_stats
            .read(|stats| {
                let kind = ScenarioPolicyResolver::resolve_kind(
                    stats.session_target_type.as_ref(),
                    stats.transport_path.as_deref(),
                );
                resolve_effective_rtt_ms(stats, kind)
            })
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod runtime_softening_tests {
    use super::{should_skip_low_value_near_deadline, should_skip_non_anchor_near_deadline};

    #[test]
    fn low_value_near_deadline_guard_triggers_before_hard_late() {
        assert!(should_skip_low_value_near_deadline(
            1_032.0,
            Some(1_040.0),
            12.0,
        ));
        assert!(!should_skip_low_value_near_deadline(
            1_020.0,
            Some(1_040.0),
            12.0,
        ));
    }

    #[test]
    fn supply_near_deadline_guard_triggers_with_tighter_window() {
        assert!(should_skip_non_anchor_near_deadline(
            1_035.0,
            Some(1_040.0),
            6.0,
        ));
        assert!(!should_skip_non_anchor_near_deadline(
            1_030.0,
            Some(1_040.0),
            6.0,
        ));
    }
}

fn classify_repair_value_tier(
    budget_context: FrameBudgetContext,
    frame_is_keyframe: bool,
    cloud_startup_mode: bool,
) -> FrameBudgetLinkValue {
    if frame_is_keyframe {
        return FrameBudgetLinkValue::Anchor;
    }
    match budget_context.dynamic_repair_value_tier() {
        DynamicRepairValueTier::Anchor => FrameBudgetLinkValue::Anchor,
        DynamicRepairValueTier::Continuation | DynamicRepairValueTier::Supply => {
            FrameBudgetLinkValue::Supply
        }
        DynamicRepairValueTier::Disposable => {
            if budget_context.prefers_low_value_skip() && !cloud_startup_mode {
                FrameBudgetLinkValue::Disposable
            } else {
                FrameBudgetLinkValue::Supply
            }
        }
    }
}

fn should_skip_low_value_near_deadline(
    estimated_recovery_arrival_ms: f64,
    deadline_at_ms: Option<f64>,
    guard_ms: f64,
) -> bool {
    deadline_at_ms
        .is_some_and(|deadline_at_ms| estimated_recovery_arrival_ms + guard_ms >= deadline_at_ms)
}

fn sample_loss_repairability(
    frame_is_keyframe: bool,
    frame_importance: &str,
    oos_active: bool,
) -> f64 {
    let base = if frame_is_keyframe {
        0.95
    } else {
        match frame_importance {
            "anchor" | "keyframe" => 0.95,
            "supply" | "reference" | "continuation" => 0.82,
            _ => 0.45,
        }
    };
    if oos_active {
        (base - OOS_REPAIRABILITY_PENALTY).max(0.2)
    } else {
        base
    }
}

fn should_skip_non_anchor_near_deadline(
    estimated_recovery_arrival_ms: f64,
    deadline_at_ms: Option<f64>,
    guard_ms: f64,
) -> bool {
    deadline_at_ms
        .is_some_and(|deadline_at_ms| estimated_recovery_arrival_ms + guard_ms >= deadline_at_ms)
}

pub(super) fn wrapping_sequence_range(start: u16, end_exclusive: u16) -> Vec<u16> {
    let mut sequences = Vec::new();
    let mut cursor = start;
    while cursor != end_exclusive {
        sequences.push(cursor);
        cursor = cursor.wrapping_add(1);
    }
    sequences
}

#[cfg(test)]
#[path = "nack_maintenance.test.rs"]
mod tests;
