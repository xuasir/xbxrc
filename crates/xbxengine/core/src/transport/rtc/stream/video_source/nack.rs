use xbxengine_protocol::XbxEngineTargetTypeDto;

use rtc_rtcp::transport_feedbacks::transport_layer_nack::{
    nack_pairs_from_sequence_numbers, TransportLayerNack,
};
use rtc_shared::marshal::{Marshal, MarshalSize};

use crate::media::video::ingress::budget::{FrameBudgetContext, FrameBudgetWindowSource};
use crate::media::video::types::FrameRecoveryDisposition;
use crate::transport::rtc::stream::nack_scheduler::{
    NackBatch, NackObservePolicy, PacketRecoveryDisposition, ResolvedNack, SkippedNackBatch,
};
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState,
    XbxEngineVideoNackObservation,
};

use super::{
    capitalize_reason, nack_policy::*, now_ms_f64, FrameValue, RecentRtpPacket,
    RtcVideoFrameSource, TransportObservation,
};
use crate::media::video::ingress::budget::{DynamicRepairValueTier, FrameBudgetLinkValue};

const DISPLAY_STARVED_LOW_VALUE_PRESENT_STALE_MS: f64 = 400.0;
const DISPLAY_STARVED_LOW_VALUE_NO_PENDING_STREAK_MIN: u32 = 24;
const LOW_VALUE_NEAR_DEADLINE_GUARD_MS: f64 = 12.0;
const SUPPLY_NEAR_DEADLINE_GUARD_MS: f64 = 6.0;
const CLEAN_ANCHOR_TRANSPORT_SUPPLY_WINDOW_MS: f64 = 320.0;
const CLEAN_ANCHOR_TRANSPORT_SUPPLY_FRESH_MEDIA_MS: f64 = 320.0;

fn gap_expired_skipped_anchor_failure_reason(
    frame_unrecoverable_reason: Option<&'static str>,
) -> XbxEngineAnchorCandidateFailureReason {
    match frame_unrecoverable_reason {
        Some("referenceChainUnrecoverable") => {
            XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
        }
        Some("sampleLossLowRepairability" | "sampleLossReferenceLowRepairability") => {
            XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
        }
        Some("cloudHighRttLowValueAdmission") => {
            XbxEngineAnchorCandidateFailureReason::TransportLowValueCloudHighRttAdmission
        }
        Some("displayStarvedLowValueAdmission") => {
            XbxEngineAnchorCandidateFailureReason::TransportLowValueDisplayStarvedAdmission
        }
        Some("estimatedArrivalNearDeadlineSupplyRecovery") => {
            XbxEngineAnchorCandidateFailureReason::TransportTimingNearDeadlineSupplyRecovery
        }
        Some("deadline") => XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline,
        _ => XbxEngineAnchorCandidateFailureReason::Unknown,
    }
}

/// transport 路径上仅有显式 keyframe 标记才写入 gap 的媒体证据 importance。
pub(super) fn gap_transport_evidence(frame_is_keyframe: Option<bool>) -> &'static str {
    if frame_is_keyframe == Some(true) {
        "anchor"
    } else {
        "unknown"
    }
}

/// 仅这些 reason 允许在 timeline 未记坏链时仍由 `SkippedChainBroken` 触发 reference 恢复。
fn nack_reference_chain_recovery_evidence(reason: Option<&'static str>) -> bool {
    matches!(
        reason,
        Some(
            "awaitingRecoveryAnchor"
                | "localBackpressureDeltaGap"
                | "sampleLossReferenceLowRepairability"
                | "referenceChainUnrecoverable"
                | "deadlineExceededBeforeAdmission"
                | "estimatedArrivalPastDeadline"
        )
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportRepairPhase {
    Startup,
    Recovery,
    Steady,
}

impl RtcVideoFrameSource {
    fn should_escalate_sample_loss_to_chain_broken(
        &self,
        source: &'static str,
        value_tier: FrameBudgetLinkValue,
        repairability: Option<f64>,
    ) -> bool {
        if source != "sampleLoss" {
            return false;
        }
        let Some(repairability) = repairability else {
            return false;
        };
        matches!(value_tier, FrameBudgetLinkValue::Supply) && repairability <= 0.45
    }

    fn transport_nack_repair_phase(&self) -> TransportRepairPhase {
        if self.is_cloud_startup_transport_profile() {
            TransportRepairPhase::Startup
        } else if self.transport_nack_recovery_pressure_active() {
            TransportRepairPhase::Recovery
        } else {
            TransportRepairPhase::Steady
        }
    }

    fn transport_nack_recovery_pressure_active(&self) -> bool {
        use crate::transport::rtc::recovery::contract::{
            derive_gap_severity_from_timeline_observation,
            gap_severity_indicates_transport_recovery_pressure,
        };
        if self.is_blocking_non_keyframe_admission() {
            return true;
        }
        self.runtime_stats
            .read(|stats| {
                stats
                    .latest_video_timeline_observation
                    .as_ref()
                    .is_some_and(|timeline| {
                        gap_severity_indicates_transport_recovery_pressure(
                            derive_gap_severity_from_timeline_observation(timeline),
                        )
                    })
                    || matches!(
                        stats.video_owner_state.as_deref(),
                        Some("rebuilding-supply" | "supply-starved")
                    )
            })
            .unwrap_or(false)
    }

    fn transport_nack_window_source(&self) -> FrameBudgetWindowSource {
        if self.transport_nack_recovery_pressure_active() {
            FrameBudgetWindowSource::Recovery
        } else {
            FrameBudgetWindowSource::Transport
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
        let pending_before = self.nack_scheduler.pending_count();
        let missing_sequences = self.nack_window.missing_seq_numbers(self.nack_skip_last_n);
        let stale_sequences = self
            .nack_scheduler
            .prune_rtp_window_pending_not_missing(&missing_sequences);
        if !stale_sequences.is_empty() {
            crate::xbx_log_info!(
                "[RtcVideoFrameSource] prune stale rtpWindow pending count={} skip_last_n={}",
                stale_sequences.len(),
                self.nack_skip_last_n
            );
        }
        let frame_value = self.current_transport_frame_value_for_transport_gap(now_ms);
        let cloud_mode = self.is_cloud_transport_profile();
        let startup_mode = self.is_cloud_startup_transport_profile();
        let cloud_rtt_ms = self.cloud_nack_rtt_ms();
        let base_budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.is_blocking_non_keyframe_admission(),
            Some(cloud_rtt_ms),
            None,
            None,
            startup_mode,
            self.transport_nack_window_source(),
        );
        let deadline_at_ms = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            self.transport_deadline_tracker
                .next_transport_deadline_with_context_at_ms(
                    now_ms,
                    frame_value,
                    base_budget_context,
                ),
            cloud_mode,
            startup_mode,
            Some(cloud_rtt_ms),
        );
        let policy = rtp_window_nack_policy(
            frame_value,
            base_budget_context,
            deadline_at_ms,
            cloud_mode,
            startup_mode,
            Some(cloud_rtt_ms),
        );
        self.timeline_state.mark_gap_reorder_pending(
            &missing_sequences,
            now_ms,
            None,
            policy.frame_importance,
            gap_transport_evidence(policy.frame_is_keyframe),
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-reorder-pending",
                Some(sequence),
                None,
                now_ms,
            );
        }
        let policy = self.with_cloud_latency_admission_policy(policy, now_ms, None);
        let (initial_batch, skipped_batch) = self
            .nack_scheduler
            .observe_missing_sequences_with_policy(&missing_sequences, now_ms, policy);
        if let Some(skipped) = skipped_batch.as_ref() {
            let chain_broken = self.timeline_state.mark_gap_expired(
                &skipped.sequences,
                now_ms,
                skipped.frame_rtp_timestamp,
                skipped.frame_importance,
                gap_transport_evidence(skipped.frame_is_keyframe),
                skipped.frame_unrecoverable_reason,
            );
            if let Some(sequence) = skipped.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-expired-skipped",
                    Some(sequence),
                    skipped.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.record_anchor_candidate_ledger(
                skipped.frame_rtp_timestamp,
                "gap-expired-skipped",
                XbxEngineAnchorCandidateState::Rejected,
                Some(gap_expired_skipped_anchor_failure_reason(
                    skipped.frame_unrecoverable_reason,
                )),
                now_ms,
            );
            self.record_nack_skipped(skipped, now_ms);
            self.maybe_handle_chain_broken(skipped, now_ms, chain_broken);
        }
        if let Some(initial_batch) = initial_batch {
            self.timeline_state.mark_gap_repair_in_flight(
                &initial_batch.sequences,
                now_ms,
                initial_batch.frame_rtp_timestamp,
                initial_batch.frame_importance,
                gap_transport_evidence(initial_batch.frame_is_keyframe),
            );
            if let Some(sequence) = initial_batch.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-repair-in-flight",
                    Some(sequence),
                    initial_batch.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.record_anchor_candidate_ledger(
                initial_batch.frame_rtp_timestamp,
                "gap-repair-in-flight",
                XbxEngineAnchorCandidateState::AwaitingRecovery,
                Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
                now_ms,
            );
            let inserted_count = self
                .nack_scheduler
                .pending_count()
                .saturating_sub(pending_before)
                .min(u16::MAX as usize) as u16;
            if inserted_count > 0 {
                self.record_packet_gap_observation(
                    &missing_sequences,
                    inserted_count,
                    now_ms,
                    "rtpWindow",
                    None,
                    None,
                    None,
                    None,
                    None,
                );
                self.runtime_stats
                    .add_inbound_video_packet_loss_estimate(inserted_count);
            }
            let _ = self.send_nack_batch("sent", &initial_batch, now_ms).await;
        }

        // 更新NACK调度器的网络状态（用于动态预算调整）
        let (rtt_ms, loss_rate) = self
            .runtime_stats
            .read(|stats| (stats.video_rtt_ms, Some(stats.inbound_video_loss_ratio_1s)))
            .unwrap_or((None, None));
        self.nack_scheduler.update_network_stats(rtt_ms, loss_rate);

        let poll_result = self.nack_scheduler.poll(now_ms);
        for expired_batch in poll_result.expired_batches {
            let chain_broken = self.timeline_state.mark_gap_expired(
                &expired_batch.sequences,
                now_ms,
                expired_batch.frame_rtp_timestamp,
                expired_batch.frame_importance,
                gap_transport_evidence(expired_batch.frame_is_keyframe),
                expired_batch.frame_unrecoverable_reason,
            );
            if let Some(sequence) = expired_batch.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-expired-poll",
                    Some(sequence),
                    expired_batch.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.record_frame_recovery_from_nack(
                expired_batch.frame_rtp_timestamp,
                expired_batch.frame_importance,
                expired_batch.nack_disposition,
                expired_batch.frame_playout_deadline_at_ms,
                expired_batch.frame_unrecoverable_reason,
                expired_batch.budget_context,
                now_ms,
            );
            if expired_batch.reason == "deadline" {
                let missing_packets = expired_batch.sequences.len().min(u16::MAX as usize) as u16;
                self.queue_transport_observation(TransportObservation::NackDeadlineExpired {
                    missing_packets,
                });
            }
            self.runtime_stats
                .add_video_loss_finalized(expired_batch.sequences.len());
            self.record_nack_observation(
                &format!("expired{}", capitalize_reason(&expired_batch.reason)),
                &NackBatch {
                    sequences: expired_batch.sequences.clone(),
                    retry_count: 0,
                    source: expired_batch.source,
                    frame_rtp_timestamp: expired_batch.frame_rtp_timestamp,
                    frame_is_keyframe: expired_batch.frame_is_keyframe,
                    frame_importance: expired_batch.frame_importance,
                    deadline_at_ms: expired_batch.deadline_at_ms,
                    estimated_recovery_arrival_ms: expired_batch.estimated_recovery_arrival_ms,
                    frame_playout_deadline_at_ms: expired_batch.frame_playout_deadline_at_ms,
                    nack_disposition: expired_batch.nack_disposition,
                    frame_unrecoverable_reason: expired_batch.frame_unrecoverable_reason,
                    budget_context: expired_batch.budget_context,
                },
                now_ms,
            );
            self.maybe_trigger_reference_chain_recovery(
                expired_batch.frame_rtp_timestamp,
                Some(expired_batch.sequences.as_slice()),
                now_ms,
                chain_broken,
            );
        }
        if let Some(retry_batch) = poll_result.retry_batch {
            self.timeline_state.mark_gap_repair_in_flight(
                &retry_batch.sequences,
                now_ms,
                retry_batch.frame_rtp_timestamp,
                retry_batch.frame_importance,
                gap_transport_evidence(retry_batch.frame_is_keyframe),
            );
            if let Some(sequence) = retry_batch.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-repair-retry",
                    Some(sequence),
                    retry_batch.frame_rtp_timestamp,
                    now_ms,
                );
            }
            let _ = self.send_nack_batch("sent", &retry_batch, now_ms).await;
        }
        self.runtime_stats
            .set_video_pending_missing_packets(self.nack_scheduler.pending_count());
    }

    pub(super) async fn observe_forward_gap_and_nack(
        &mut self,
        expected_sequence: u16,
        received_sequence: u16,
    ) {
        let now_ms = now_ms_f64();
        let pending_before = self.nack_scheduler.pending_count();
        let missing_sequences = wrapping_sequence_range(expected_sequence, received_sequence);
        if missing_sequences.is_empty() {
            return;
        }
        let frame_value = self.current_transport_frame_value_for_transport_gap(now_ms);
        let cloud_mode = self.is_cloud_transport_profile();
        let startup_mode = self.is_cloud_startup_transport_profile();
        let cloud_rtt_ms = self.cloud_nack_rtt_ms();
        let base_budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.is_blocking_non_keyframe_admission(),
            Some(cloud_rtt_ms),
            None,
            None,
            startup_mode,
            self.transport_nack_window_source(),
        );
        let deadline_at_ms = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            self.transport_deadline_tracker
                .next_transport_deadline_with_context_at_ms(
                    now_ms,
                    frame_value,
                    base_budget_context,
                ),
            cloud_mode,
            startup_mode,
            Some(cloud_rtt_ms),
        );
        let policy = rtp_gap_nack_policy(
            frame_value,
            base_budget_context,
            deadline_at_ms,
            cloud_mode,
            startup_mode,
            Some(cloud_rtt_ms),
        );
        self.timeline_state.observe_gap(
            &missing_sequences,
            now_ms,
            None,
            policy.frame_importance,
            gap_transport_evidence(policy.frame_is_keyframe),
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-observed-forward",
                Some(sequence),
                None,
                now_ms,
            );
        }
        self.timeline_state.mark_gap_nack_candidate(
            &missing_sequences,
            now_ms,
            None,
            policy.frame_importance,
            gap_transport_evidence(policy.frame_is_keyframe),
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-nack-candidate",
                Some(sequence),
                None,
                now_ms,
            );
        }
        let policy = self.with_cloud_latency_admission_policy(policy, now_ms, None);
        let (initial_batch, skipped_batch) = self
            .nack_scheduler
            .observe_missing_sequences_with_policy(&missing_sequences, now_ms, policy);
        if let Some(skipped) = skipped_batch.as_ref() {
            let chain_broken = self.timeline_state.mark_gap_expired(
                &skipped.sequences,
                now_ms,
                skipped.frame_rtp_timestamp,
                skipped.frame_importance,
                gap_transport_evidence(skipped.frame_is_keyframe),
                skipped.frame_unrecoverable_reason,
            );
            if let Some(sequence) = skipped.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-expired-skipped",
                    Some(sequence),
                    skipped.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.record_anchor_candidate_ledger(
                skipped.frame_rtp_timestamp,
                "gap-expired-skipped",
                XbxEngineAnchorCandidateState::Rejected,
                Some(gap_expired_skipped_anchor_failure_reason(
                    skipped.frame_unrecoverable_reason,
                )),
                now_ms,
            );
            self.record_nack_skipped(skipped, now_ms);
            self.maybe_handle_chain_broken(skipped, now_ms, chain_broken);
        }
        let Some(initial_batch) = initial_batch else {
            return;
        };
        self.timeline_state.mark_gap_repair_in_flight(
            &initial_batch.sequences,
            now_ms,
            initial_batch.frame_rtp_timestamp,
            initial_batch.frame_importance,
            gap_transport_evidence(initial_batch.frame_is_keyframe),
        );
        if let Some(sequence) = initial_batch.sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-repair-in-flight",
                Some(sequence),
                initial_batch.frame_rtp_timestamp,
                now_ms,
            );
        }
        self.record_anchor_candidate_ledger(
            initial_batch.frame_rtp_timestamp,
            "gap-repair-in-flight",
            XbxEngineAnchorCandidateState::AwaitingRecovery,
            Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
            now_ms,
        );
        let inserted_count = self
            .nack_scheduler
            .pending_count()
            .saturating_sub(pending_before)
            .min(u16::MAX as usize) as u16;
        if inserted_count > 0 {
            self.record_packet_gap_observation(
                &missing_sequences,
                inserted_count,
                now_ms,
                "rtpGap",
                None,
                None,
                None,
                None,
                None,
            );
            self.runtime_stats
                .add_inbound_video_packet_loss_estimate(inserted_count);
        }
        let _ = self.send_nack_batch("sent", &initial_batch, now_ms).await;
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

        self.rtcp_port.send_rtcp(&buf).map_err(|e| {
            crate::xbx_log_warn!(
                "[RtcVideoFrameSource] nack send failed action={} error={}",
                action,
                e
            );
            e
        })?;

        // 只在真正发送成功后记录统计
        self.runtime_stats
            .record_nack_sent(batch.sequences.len(), self.nack_scheduler.pending_count());
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
            self.nack_scheduler.pending_count(),
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

    pub(super) async fn observe_sample_loss_and_nack(
        &mut self,
        sample_rtp_timestamp: u32,
        media_dropped_packets: u16,
        frame_is_keyframe: bool,
        frame_importance: &'static str,
    ) -> bool {
        let now_ms = now_ms_f64();
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
        // 将媒体语义标签转换为恢复语义标签
        let recovery_label = recovery_label_for_media_label(frame_importance);
        let frame_value = self.merge_media_frame_value_with_recovery_timeline(
            frame_value_for_importance(recovery_label),
        );
        let window_source = self.transport_nack_window_source();
        let repairability = self.estimate_repairability(
            recovery_label,
            attributed_dropped_packets,
            missing_sequences.len().min(u16::MAX as usize) as u16,
            window_source,
            Some(sample_rtp_timestamp),
        );
        let base_deadline_at_ms = self
            .transport_deadline_tracker
            .next_transport_deadline_with_context_at_ms(
                now_ms,
                frame_value,
                FrameBudgetContext::for_transport(
                    frame_value,
                    self.is_blocking_non_keyframe_admission(),
                    Some(self.cloud_nack_rtt_ms()),
                    None,
                    None,
                    self.is_cloud_startup_transport_profile(),
                    window_source,
                ),
            );
        let deadline_at_ms =
            self.dynamic_repair_deadline(now_ms, base_deadline_at_ms, repairability);
        let budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.is_blocking_non_keyframe_admission(),
            Some(self.cloud_nack_rtt_ms()),
            None,
            Some(deadline_at_ms),
            self.is_cloud_startup_transport_profile(),
            window_source,
        );
        let policy = sample_loss_nack_policy(
            sample_rtp_timestamp,
            frame_is_keyframe,
            budget_context,
            deadline_at_ms,
            repairability,
            self.is_cloud_transport_profile(),
            self.is_cloud_startup_transport_profile(),
            Some(self.cloud_nack_rtt_ms()),
        );
        self.timeline_state.observe_gap(
            &missing_sequences,
            now_ms,
            Some(sample_rtp_timestamp),
            recovery_label,
            gap_transport_evidence(Some(frame_is_keyframe)),
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-observed-sample-loss",
                Some(sequence),
                Some(sample_rtp_timestamp),
                now_ms,
            );
        }
        self.timeline_state.mark_gap_nack_candidate(
            &missing_sequences,
            now_ms,
            Some(sample_rtp_timestamp),
            frame_importance,
            gap_transport_evidence(Some(frame_is_keyframe)),
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-nack-candidate",
                Some(sequence),
                Some(sample_rtp_timestamp),
                now_ms,
            );
        }
        let policy = self.with_cloud_latency_admission_policy(policy, now_ms, Some(repairability));
        let pending_before = self.nack_scheduler.pending_count();
        let (batch, skipped_batch) = self.nack_scheduler.observe_missing_sequences_with_policy(
            &missing_sequences,
            now_ms,
            policy,
        );
        if let Some(skipped) = skipped_batch.as_ref() {
            let chain_broken = self.timeline_state.mark_gap_expired(
                &skipped.sequences,
                now_ms,
                skipped.frame_rtp_timestamp,
                skipped.frame_importance,
                gap_transport_evidence(skipped.frame_is_keyframe),
                skipped.frame_unrecoverable_reason,
            );
            if let Some(sequence) = skipped.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-expired-skipped",
                    Some(sequence),
                    skipped.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.record_anchor_candidate_ledger(
                skipped.frame_rtp_timestamp,
                "gap-expired-skipped",
                XbxEngineAnchorCandidateState::Rejected,
                Some(gap_expired_skipped_anchor_failure_reason(
                    skipped.frame_unrecoverable_reason,
                )),
                now_ms,
            );
            self.record_nack_skipped(skipped, now_ms);
            self.maybe_handle_chain_broken(skipped, now_ms, chain_broken);
        }
        let Some(batch) = batch else {
            return false;
        };
        self.timeline_state.mark_gap_repair_in_flight(
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
            Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
            now_ms,
        );
        let inserted_count = self
            .nack_scheduler
            .pending_count()
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
        Some(FrameRecoveryDisposition::UnrecoverableReferenceChain)
    }

    /// Cloud 路径下在 deadline / RTT slack 内尽量尝试 NACK（预算充裕指时间窗与优先级足够）。
    /// 若 `blocking_non_keyframe_admission` 或 `prefers_chain_broken()` 为真，会故意对非锚点缺口标
    /// `SkippedChainBroken` / `awaitingRecoveryAnchor`——这是**策略上**等 IDR 而非「NACK 排队链条过长」。
    fn with_cloud_latency_admission_policy(
        &self,
        mut policy: NackObservePolicy,
        now_ms: f64,
        repairability: Option<f64>,
    ) -> NackObservePolicy {
        let repair_phase = self.transport_nack_repair_phase();
        if self.should_soften_display_starved_low_value_gap(
            policy.frame_importance,
            policy.frame_is_keyframe,
            policy.budget_context,
            now_ms,
        ) {
            policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
            if policy.frame_unrecoverable_reason.is_none() {
                policy.frame_unrecoverable_reason = Some("displayStarvedLowValueAdmission");
            }
            return policy;
        }

        if !self.is_cloud_transport_profile() {
            return policy;
        }
        let retry_interval_ms = policy.retry_interval_ms.unwrap_or(16) as f64;
        let estimated_recovery_arrival_ms = now_ms
            + (self.cloud_nack_rtt_ms().max(0.0) * 0.5)
                .max(self.nack_recovery_ewma_ms.max(0.0))
                .max(retry_interval_ms);
        policy.estimated_recovery_arrival_ms = Some(estimated_recovery_arrival_ms);
        let frame_value = frame_value_for_importance(policy.frame_importance);
        let window_source = policy.budget_context.window_source;
        policy.budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.is_blocking_non_keyframe_admission(),
            Some(self.cloud_nack_rtt_ms()),
            Some(estimated_recovery_arrival_ms),
            policy.deadline_at_ms,
            self.is_cloud_startup_transport_profile(),
            window_source,
        );
        if !policy.frame_is_keyframe.unwrap_or(false) {
            policy.frame_importance = policy.budget_context.recovery_value_tier();
        }
        policy.priority = policy.budget_context.repair_priority(frame_value);

        let value_tier = classify_repair_value_tier(
            policy.budget_context,
            policy.frame_is_keyframe.unwrap_or(false),
            self.is_cloud_startup_transport_profile(),
        );
        let frame_seen_oos = policy
            .frame_rtp_timestamp
            .is_some_and(|timestamp| self.frame_seen_oos(timestamp));
        let oos_recently_active = self.oos_recently_active(now_ms);
        let oos_signal_active = frame_seen_oos || oos_recently_active;
        if oos_signal_active && !policy.frame_is_keyframe.unwrap_or(false) {
            policy.priority = policy.priority.saturating_sub(1).max(1);
            // OOS + LowValue + disposable：降级为 SkippedLowValue，但必须先让 prefers_chain_broken
            // 检查通过，避免跳过 SkippedChainBroken 语义（chain broken 需要触发恢复流程）。
            if matches!(value_tier, FrameBudgetLinkValue::Disposable)
                && policy.frame_importance == "disposable"
                && policy.nack_disposition == PacketRecoveryDisposition::Attempted
                && !policy.budget_context.prefers_chain_broken()
            {
                policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
                if policy.frame_unrecoverable_reason.is_none() {
                    policy.frame_unrecoverable_reason = Some("oosLowValueAdmission");
                }
                return policy;
            }
        }

        if policy.budget_context.prefers_chain_broken()
            && !matches!(value_tier, FrameBudgetLinkValue::Anchor)
        {
            policy.nack_disposition = PacketRecoveryDisposition::SkippedChainBroken;
            if policy.frame_unrecoverable_reason.is_none() {
                policy.frame_unrecoverable_reason = Some(
                    if matches!(value_tier, FrameBudgetLinkValue::Disposable)
                        && policy.frame_importance == "disposable"
                    {
                        "localBackpressureDeltaGap"
                    } else {
                        "awaitingRecoveryAnchor"
                    },
                );
            }
            return policy;
        }

        if self.should_escalate_sample_loss_to_chain_broken(
            policy.source,
            value_tier,
            repairability,
        ) {
            policy.nack_disposition = PacketRecoveryDisposition::SkippedChainBroken;
            if policy.frame_unrecoverable_reason.is_none() {
                policy.frame_unrecoverable_reason = Some("sampleLossReferenceLowRepairability");
            }
            return policy;
        }

        if self.is_cloud_high_rtt_path()
            && matches!(value_tier, FrameBudgetLinkValue::Disposable)
            && policy.budget_context.prefers_low_value_skip()
        {
            policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
            if policy.frame_unrecoverable_reason.is_none() {
                policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");
            }
            return policy;
        }

        if matches!(value_tier, FrameBudgetLinkValue::Disposable)
            && should_skip_low_value_near_deadline(
                estimated_recovery_arrival_ms,
                policy.deadline_at_ms,
                LOW_VALUE_NEAR_DEADLINE_GUARD_MS,
            )
        {
            policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
            if policy.frame_unrecoverable_reason.is_none() {
                policy.frame_unrecoverable_reason = Some("estimatedArrivalNearDeadlineLowValue");
            }
            return policy;
        }

        if matches!(value_tier, FrameBudgetLinkValue::Supply)
            && should_skip_non_anchor_near_deadline(
                estimated_recovery_arrival_ms,
                policy.deadline_at_ms,
                SUPPLY_NEAR_DEADLINE_GUARD_MS,
            )
        {
            if repair_phase == TransportRepairPhase::Recovery {
                policy.nack_disposition = PacketRecoveryDisposition::SkippedTooLate;
                if policy.frame_unrecoverable_reason.is_none() {
                    policy.frame_unrecoverable_reason =
                        Some("estimatedArrivalNearDeadlineSupplyRecovery");
                }
                return policy;
            }
            if repair_phase == TransportRepairPhase::Steady {
                // 与 Startup 一致：先尝试 NACK，由 scheduler deadline/max_age/retry 收口。
                return policy;
            }
            // 建链期对 supply/reference 给予更宽的补缺窗口，避免刚起流就过早切到 keyframe 恢复。
            return policy;
        }

        if let Some(deadline_at_ms) = policy.deadline_at_ms {
            if estimated_recovery_arrival_ms > deadline_at_ms {
                policy.nack_disposition = PacketRecoveryDisposition::SkippedTooLate;
                if policy.frame_unrecoverable_reason.is_none() {
                    policy.frame_unrecoverable_reason = Some("estimatedArrivalPastDeadline");
                }
                return policy;
            }
        }

        policy
    }

    fn should_soften_display_starved_low_value_gap(
        &self,
        frame_importance: &'static str,
        frame_is_keyframe: Option<bool>,
        budget_context: FrameBudgetContext,
        now_ms: f64,
    ) -> bool {
        if frame_importance != "disposable" || frame_is_keyframe.unwrap_or(false) {
            return false;
        }
        if !matches!(
            budget_context.failure_cost,
            crate::media::video::ingress::budget::FrameBudgetFailureCost::LocalDrop
        ) {
            return false;
        }
        if !matches!(
            budget_context.link_value,
            crate::media::video::ingress::budget::FrameBudgetLinkValue::Disposable
        ) {
            return false;
        }
        self.runtime_stats
            .read(|stats| {
                should_soften_display_starved_low_value_gap_from_runtime(
                    stats.host_no_pending_pressure_level.as_deref(),
                    stats.host_no_pending_streak,
                    stats.latest_video_host_present_time_ms,
                    now_ms,
                )
            })
            .unwrap_or(false)
    }

    fn is_cloud_high_rtt_path(&self) -> bool {
        self.cloud_nack_rtt_ms() >= 120.0
    }

    fn maybe_handle_chain_broken(
        &mut self,
        skipped: &SkippedNackBatch,
        now_ms: f64,
        chain_broken: bool,
    ) {
        let trigger = match skipped.nack_disposition {
            PacketRecoveryDisposition::SkippedLowValue
            | PacketRecoveryDisposition::SkippedTooLate => chain_broken,
            PacketRecoveryDisposition::SkippedChainBroken => {
                chain_broken
                    || nack_reference_chain_recovery_evidence(skipped.frame_unrecoverable_reason)
            }
            PacketRecoveryDisposition::Attempted => return,
        };
        if !trigger {
            return;
        }
        self.maybe_trigger_reference_chain_recovery(
            skipped.frame_rtp_timestamp,
            Some(skipped.sequences.as_slice()),
            now_ms,
            true,
        );
    }

    fn maybe_trigger_reference_chain_recovery(
        &mut self,
        frame_rtp_timestamp: Option<u32>,
        sequences: Option<&[u16]>,
        now_ms: f64,
        chain_broken: bool,
    ) {
        if !chain_broken || self.is_blocking_non_keyframe_admission() {
            return;
        }
        self.timeline_state.on_chain_broken();
        if let Some(sequence) = sequences.and_then(|value| value.first().copied()) {
            self.record_video_timeline_observation(
                "chain-broken",
                Some(sequence),
                frame_rtp_timestamp,
                now_ms,
            );
        }
        let mut allow_non_anchor_soft_request = false;
        if let Some(flushed_batch) = self
            .nack_scheduler
            .flush_non_keyframe_pending("flushedAfterChainBrokenAdmission")
        {
            allow_non_anchor_soft_request = flushed_batch.frame_importance == "disposable";
            self.timeline_state.mark_gap_expired(
                &flushed_batch.sequences,
                now_ms,
                flushed_batch.frame_rtp_timestamp,
                flushed_batch.frame_importance,
                gap_transport_evidence(flushed_batch.frame_is_keyframe),
                flushed_batch.frame_unrecoverable_reason,
            );
            if let Some(sequence) = flushed_batch.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-expired-chain-flush",
                    Some(sequence),
                    flushed_batch.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.record_frame_recovery_from_nack(
                flushed_batch.frame_rtp_timestamp,
                flushed_batch.frame_importance,
                flushed_batch.nack_disposition,
                flushed_batch.frame_playout_deadline_at_ms,
                flushed_batch.frame_unrecoverable_reason,
                flushed_batch.budget_context,
                now_ms,
            );
            self.runtime_stats
                .add_video_loss_finalized(flushed_batch.sequences.len());
            self.record_nack_observation(
                "expiredChainBroken",
                &NackBatch {
                    sequences: flushed_batch.sequences,
                    retry_count: 0,
                    source: flushed_batch.source,
                    frame_rtp_timestamp: flushed_batch.frame_rtp_timestamp,
                    frame_is_keyframe: flushed_batch.frame_is_keyframe,
                    frame_importance: flushed_batch.frame_importance,
                    deadline_at_ms: flushed_batch.deadline_at_ms,
                    estimated_recovery_arrival_ms: flushed_batch.estimated_recovery_arrival_ms,
                    frame_playout_deadline_at_ms: flushed_batch.frame_playout_deadline_at_ms,
                    nack_disposition: flushed_batch.nack_disposition,
                    frame_unrecoverable_reason: flushed_batch.frame_unrecoverable_reason,
                    budget_context: flushed_batch.budget_context,
                },
                now_ms,
            );
        }
        let should_soft_request = self
            .runtime_stats
            .read(|stats| {
                Self::should_soft_request_recovery_keyframe(
                    stats,
                    now_ms,
                    None,
                    false,
                    allow_non_anchor_soft_request,
                    false,
                )
            })
            .unwrap_or(false);
        if should_soft_request {
            self.request_recovery_keyframe_soft_from_source(
                "chain-recovery-anchor-requested",
                frame_rtp_timestamp,
                now_ms,
            );
        } else {
            self.request_recovery_keyframe_from_source(
                "chain-recovery-anchor-requested",
                frame_rtp_timestamp,
                now_ms,
            );
            self.set_is_blocking_non_keyframe_admission(true);
        }
    }

    fn collect_recent_missing_sequences(&self, media_dropped_packets: u16) -> Vec<u16> {
        let mut missing = self.nack_window.missing_seq_numbers(0);
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
            .nack_window
            .missing_seq_numbers_in_range(start, end_exclusive);
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

    fn current_transport_frame_value_for_transport_gap(&self, now_ms: f64) -> FrameValue {
        let last_value = self.last_submitted_frame_value;
        if last_value.is_sync_point() || last_value.refresh_boost {
            return last_value;
        }
        if self.is_blocking_non_keyframe_admission() {
            return last_value;
        }

        let should_promote = self
            .runtime_stats
            .read(|stats| {
                if stats.session_target_type != Some(XbxEngineTargetTypeDto::Cloud) {
                    return false;
                }
                let has_current_clean_anchor = stats
                    .video_anchor_clean_epoch
                    .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                    && stats.video_anchor_clean_source_event.as_deref()
                        == Some("chain-clean-anchor-submitted");
                if !has_current_clean_anchor {
                    return false;
                }
                let clean_anchor_recent =
                    stats
                        .video_anchor_clean_observed_at_ms
                        .is_some_and(|at_ms| {
                            (now_ms - at_ms).max(0.0) <= CLEAN_ANCHOR_TRANSPORT_SUPPLY_WINDOW_MS
                        });
                if !clean_anchor_recent {
                    return false;
                }
                let present_fresh = stats
                    .latest_video_host_present_time_ms
                    .is_some_and(|at_ms| {
                        (now_ms - at_ms).max(0.0) <= CLEAN_ANCHOR_TRANSPORT_SUPPLY_FRESH_MEDIA_MS
                    });
                let decode_fresh = stats.latest_video_decode_ok_time_ms.is_some_and(|at_ms| {
                    (now_ms - at_ms).max(0.0) <= CLEAN_ANCHOR_TRANSPORT_SUPPLY_FRESH_MEDIA_MS
                });
                present_fresh || decode_fresh
            })
            .unwrap_or(false);

        let after_cloud = if should_promote {
            // clean anchor 后的短窗内，transport gap 仍可能落在刚建立的新参考链上；
            // 这里把 plain delta 提升为 refresh_boost，复用既有 Supply 预算语义。
            FrameValue::new(false, true, last_value.payload_size_bytes)
        } else {
            last_value
        };
        self.merge_media_frame_value_with_recovery_timeline(after_cloud)
    }

    /// 将 `latest_video_timeline_observation` 推导的统一 gap/帧价值并入媒体层 NACK 预算，避免与 recovery 合同漂移。
    fn merge_media_frame_value_with_recovery_timeline(&self, base: FrameValue) -> FrameValue {
        use crate::transport::rtc::recovery::contract::{
            derive_gap_severity_from_timeline_observation, frame_value_from_gap_severity,
            gap_severity_indicates_transport_recovery_pressure,
            media_frame_value_from_recovery_semantics, GapSeverity,
        };
        self.runtime_stats
            .read(|stats| {
                let Some(timeline) = stats.latest_video_timeline_observation.as_ref() else {
                    return base;
                };
                let gs = derive_gap_severity_from_timeline_observation(timeline);
                // transport 反灌只提升到 supply/reference，避免闭环把上下文再抬成 anchor。
                let gs = match gs {
                    GapSeverity::ChainBroken | GapSeverity::AnchorGap => GapSeverity::ReferenceGap,
                    other => other,
                };
                if !gap_severity_indicates_transport_recovery_pressure(gs) {
                    return base;
                }
                let Some(semantic) = frame_value_from_gap_severity(gs) else {
                    return base;
                };
                let hinted =
                    media_frame_value_from_recovery_semantics(semantic, base.payload_size_bytes);
                if hinted.is_sync_point() {
                    // transport gap 没有真实帧归属，不能把恢复语义直接抬成媒体 sync-point；
                    // 否则会把普通丢包伪装成 keyframe gap，沿着 NACK/timeline 路径过早打断链。
                    return FrameValue::new(false, true, base.payload_size_bytes);
                }
                if hinted.refresh_boost && !base.is_sync_point() {
                    return hinted;
                }
                base
            })
            .unwrap_or(base)
    }

    fn estimate_repairability(
        &self,
        frame_importance: &'static str,
        media_dropped_packets: u16,
        missing_sequence_count: u16,
        window_source: FrameBudgetWindowSource,
        frame_rtp_timestamp: Option<u32>,
    ) -> f64 {
        let base = match frame_importance {
            "anchor" => 0.95,
            "supply" => 0.8,
            _ => 0.62,
        };
        // 动态 repairability：综合帧价值、缺包规模、当前恢复状态与历史恢复质量。
        let missing_ratio = missing_sequence_count as f64 / f64::from(media_dropped_packets.max(1));
        let burst_penalty = f64::from(self.sample_loss_burst_count.min(6)) * 0.04;
        let late_penalty = self.nack_late_ewma * 0.35;
        let missing_penalty = (missing_ratio - 1.0).max(0.0) * 0.08;
        let recovery_bonus = if self.nack_recovery_ewma_ms <= 16.0 {
            0.08
        } else if self.nack_recovery_ewma_ms <= 24.0 {
            0.04
        } else {
            0.0
        };
        let phase_adjustment = match self.transport_nack_repair_phase() {
            TransportRepairPhase::Startup => 0.06,
            TransportRepairPhase::Recovery => -0.04,
            TransportRepairPhase::Steady => 0.0,
        };
        let waiting_penalty = if self.is_blocking_non_keyframe_admission() {
            0.06
        } else {
            0.0
        };
        let window_penalty = if matches!(window_source, FrameBudgetWindowSource::Recovery) {
            0.04
        } else {
            0.0
        };
        let oos_penalty = if frame_rtp_timestamp
            .is_some_and(|timestamp| self.frame_seen_oos(timestamp))
            || self.oos_recently_active(now_ms_f64())
        {
            OOS_REPAIRABILITY_PENALTY
        } else {
            0.0
        };
        let head_missing_penalty = if frame_rtp_timestamp
            .is_some_and(|timestamp| self.frame_seen_head_missing(timestamp))
            || self.head_missing_recently_active(now_ms_f64())
        {
            0.12
        } else {
            0.0
        };
        (base + recovery_bonus + phase_adjustment
            - burst_penalty
            - late_penalty
            - missing_penalty
            - waiting_penalty
            - window_penalty
            - oos_penalty
            - head_missing_penalty)
            .clamp(0.25, 1.0)
    }

    fn dynamic_repair_deadline(
        &self,
        now_ms: f64,
        base_deadline_at_ms: f64,
        repairability: f64,
    ) -> f64 {
        let mut base_window_ms = (base_deadline_at_ms - now_ms).max(10.0);
        let scale = if self.is_cloud_startup_transport_profile() {
            // startup + cloud 需要更宽的首洞修复窗口，避免刚出画就把恢复链判死。
            let rtt_ms = self.cloud_nack_rtt_ms();
            base_window_ms = base_window_ms
                .max(rtt_ms + cloud_nack_rtt_margin_ms(true, Some(rtt_ms)))
                .max(CLOUD_STARTUP_HEAD_HOLE_DEADLINE_FLOOR_MS);
            (1.05 + repairability * 0.7).clamp(1.05, 1.7)
        } else if self.is_cloud_transport_profile() {
            // cloud 场景直接按运行时 RTT 放宽恢复窗口，不再额外加硬下限。
            let rtt_ms = self.cloud_nack_rtt_ms();
            base_window_ms =
                base_window_ms.max(rtt_ms + cloud_nack_rtt_margin_ms(false, Some(rtt_ms)));
            (0.95 + repairability * 0.65).clamp(0.95, 1.55)
        } else {
            (0.75 + repairability * 0.55).clamp(0.75, 1.3)
        };
        now_ms + (base_window_ms * scale)
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
            .read(|stats| stats.video_rtt_ms.unwrap_or(0.0))
            .unwrap_or(0.0)
    }
}

fn should_soften_display_starved_low_value_gap_from_runtime(
    host_no_pending_pressure_level: Option<&str>,
    host_no_pending_streak: u32,
    latest_video_host_present_time_ms: Option<f64>,
    now_ms: f64,
) -> bool {
    if host_no_pending_pressure_level != Some("critical") {
        return false;
    }
    if host_no_pending_streak < DISPLAY_STARVED_LOW_VALUE_NO_PENDING_STREAK_MIN {
        return false;
    }
    latest_video_host_present_time_ms.is_some_and(|present_at_ms| {
        (now_ms - present_at_ms).max(0.0) >= DISPLAY_STARVED_LOW_VALUE_PRESENT_STALE_MS
    })
}

#[cfg(test)]
mod runtime_softening_tests {
    use super::{
        should_skip_low_value_near_deadline, should_skip_non_anchor_near_deadline,
        should_soften_display_starved_low_value_gap_from_runtime,
    };

    #[test]
    fn display_starved_runtime_marks_low_value_gap_soft() {
        assert!(should_soften_display_starved_low_value_gap_from_runtime(
            Some("critical"),
            48,
            Some(1_000.0),
            1_450.0,
        ));
    }

    #[test]
    fn non_critical_or_fresh_present_does_not_soften_gap() {
        assert!(!should_soften_display_starved_low_value_gap_from_runtime(
            Some("elevated"),
            48,
            Some(1_000.0),
            1_450.0,
        ));
        assert!(!should_soften_display_starved_low_value_gap_from_runtime(
            Some("critical"),
            48,
            Some(1_200.0),
            1_450.0,
        ));
    }

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
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::transport::rtc::stream::sink::RtcRtcpSendPort;
    use crate::transport::rtc::stream::video_source::NackSchedulerConfig;
    use bytes::Bytes;
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    #[test]
    fn cloud_nack_windows_follow_rtt_without_floor() {
        let now_ms = 1_000.0;
        let base_deadline_at_ms = 1_120.0;

        let adjusted_deadline = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            base_deadline_at_ms,
            true,
            false,
            Some(90.0),
        );
        let adjusted_max_age = cloud_nack_max_age_ms(100, true, false, Some(90.0));

        assert_eq!(adjusted_deadline, 1_170.0);
        assert_eq!(adjusted_max_age, 170);
    }

    #[test]
    fn non_cloud_nack_windows_remain_unchanged() {
        let now_ms = 1_000.0;
        let base_deadline_at_ms = 1_120.0;

        let adjusted_deadline = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            base_deadline_at_ms,
            false,
            false,
            Some(90.0),
        );
        let adjusted_max_age = cloud_nack_max_age_ms(180, false, false, Some(90.0));

        assert_eq!(adjusted_deadline, base_deadline_at_ms);
        assert_eq!(adjusted_max_age, 180);
    }

    #[test]
    fn repair_value_tier_marks_delta_as_low_value_on_cloud_high_rtt() {
        assert_eq!(
            classify_repair_value_tier(
                FrameBudgetContext::for_transport(
                    FrameValue::new(false, false, 8 * 1024),
                    false,
                    Some(160.0),
                    Some(1_030.0),
                    Some(1_040.0),
                    false,
                    FrameBudgetWindowSource::Transport,
                ),
                false,
                false,
            ),
            FrameBudgetLinkValue::Disposable
        );
    }

    #[test]
    fn repair_value_tier_keeps_reference_as_anchor_while_waiting_keyframe() {
        assert_eq!(
            classify_repair_value_tier(
                FrameBudgetContext::for_transport(
                    FrameValue::new(false, true, 48 * 1024),
                    true,
                    Some(140.0),
                    Some(1_020.0),
                    Some(1_050.0),
                    false,
                    FrameBudgetWindowSource::Recovery,
                ),
                false,
                false,
            ),
            FrameBudgetLinkValue::Anchor
        );
    }

    #[test]
    fn repair_value_tier_marks_reference_as_supply_when_not_waiting_keyframe() {
        assert_eq!(
            classify_repair_value_tier(
                FrameBudgetContext::for_transport(
                    FrameValue::new(false, true, 48 * 1024),
                    false,
                    Some(140.0),
                    Some(1_020.0),
                    Some(1_050.0),
                    false,
                    FrameBudgetWindowSource::Transport,
                ),
                false,
                false,
            ),
            FrameBudgetLinkValue::Supply
        );
    }

    #[test]
    fn recent_clean_anchor_promotes_transport_gap_value_to_supply_on_cloud() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        source.last_submitted_frame_value = FrameValue::new(false, false, 12 * 1024);
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.transport_recovery_epoch = 7;
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(1_000.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_host_present_time_ms = Some(1_120.0);
            stats.latest_video_decode_ok_time_ms = Some(1_118.0);
        }

        let value = source.current_transport_frame_value_for_transport_gap(1_200.0);
        assert_eq!(value, FrameValue::new(false, true, 12 * 1024));
    }

    #[test]
    fn stale_clean_anchor_does_not_promote_transport_gap_value() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        source.last_submitted_frame_value = FrameValue::new(false, false, 12 * 1024);
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.transport_recovery_epoch = 7;
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(1_000.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_host_present_time_ms = Some(1_120.0);
            stats.latest_video_decode_ok_time_ms = Some(1_118.0);
        }

        let value = source.current_transport_frame_value_for_transport_gap(1_400.0);
        assert_eq!(value, FrameValue::new(false, false, 12 * 1024));
    }

    #[test]
    fn waiting_keyframe_keeps_transport_gap_value_unpromoted() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        source.last_submitted_frame_value = FrameValue::new(false, false, 12 * 1024);
        source.set_is_blocking_non_keyframe_admission(true);
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.transport_recovery_epoch = 7;
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(1_000.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_host_present_time_ms = Some(1_120.0);
            stats.latest_video_decode_ok_time_ms = Some(1_118.0);
        }

        let value = source.current_transport_frame_value_for_transport_gap(1_200.0);
        assert_eq!(value, FrameValue::new(false, false, 12 * 1024));
    }

    #[test]
    fn transport_gap_chain_broken_timeline_does_not_promote_to_pseudo_keyframe() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        source.last_submitted_frame_value = FrameValue::new(false, false, 12 * 1024);
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 6,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repair-in-flight".to_string(),
                        sequence: Some(23913),
                        frame_rtp_timestamp: None,
                        frame_importance: Some("anchor".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: 200.0,
                    }),
                    frame: Some(crate::XbxEngineVideoTimelineFrameSnapshot {
                        state: "complete-candidate".to_string(),
                        frame_rtp_timestamp: Some(2680907269),
                        is_keyframe: Some(false),
                        frame_importance: Some("disposable".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        close_reason: None,
                        observed_at_ms: 200.0,
                    }),
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "broken".to_string(),
                        reason: Some("referenceChainUnrecoverable".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 200.0,
                    },
                    observed_at_ms: 200.0,
                });
        }

        let value = source.current_transport_frame_value_for_transport_gap(220.0);
        assert_eq!(value, FrameValue::new(false, true, 12 * 1024));
        assert!(!value.is_sync_point());
    }

    #[test]
    fn transport_gap_uses_recovery_window_when_timeline_shows_recovery_pressure() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repair-in-flight".to_string(),
                        sequence: Some(33),
                        frame_rtp_timestamp: None,
                        frame_importance: Some("supply".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: 100.0,
                    }),
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "repairing".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 100.0,
                    },
                    observed_at_ms: 100.0,
                });
        }

        assert_eq!(
            source.transport_nack_window_source(),
            FrameBudgetWindowSource::Recovery
        );
    }

    #[test]
    fn cloud_latency_admission_preserves_recovery_window_for_transport_gap() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.video_rtt_ms = Some(140.0);
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 3,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repair-in-flight".to_string(),
                        sequence: Some(57),
                        frame_rtp_timestamp: None,
                        frame_importance: Some("supply".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: 140.0,
                    }),
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 140.0,
                    },
                    observed_at_ms: 140.0,
                });
        }

        let policy = rtp_gap_nack_policy(
            FrameValue::new(false, false, 12 * 1024),
            FrameBudgetContext::for_transport(
                FrameValue::new(false, false, 12 * 1024),
                false,
                Some(140.0),
                None,
                Some(220.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
            220.0,
            true,
            false,
            Some(140.0),
        );
        let resolved = source.with_cloud_latency_admission_policy(policy, 180.0, None);

        assert_eq!(
            resolved.budget_context.window_source,
            FrameBudgetWindowSource::Recovery
        );
    }

    #[test]
    fn sample_loss_low_repairability_delta_does_not_escalate_to_chain_broken() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.video_rtt_ms = Some(140.0);
        }

        let policy = sample_loss_nack_policy(
            91_200,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, false, 12 * 1024),
                false,
                Some(140.0),
                None,
                Some(190.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
            190.0,
            0.35,
            true,
            false,
            Some(140.0),
        );
        let resolved = source.with_cloud_latency_admission_policy(policy, 160.0, Some(0.35));

        assert_eq!(
            resolved.nack_disposition,
            PacketRecoveryDisposition::SkippedLowValue
        );
        assert_eq!(
            resolved.frame_unrecoverable_reason,
            Some("cloudHighRttLowValueAdmission")
        );
    }

    #[test]
    fn sample_loss_reference_low_repairability_escalates_without_waiting_deadline() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.video_rtt_ms = Some(80.0);
        }

        let policy = sample_loss_nack_policy(
            91_260,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, true, 48 * 1024),
                false,
                Some(80.0),
                None,
                Some(260.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
            260.0,
            0.4,
            true,
            false,
            Some(80.0),
        );
        let resolved = source.with_cloud_latency_admission_policy(policy, 180.0, Some(0.4));

        assert_eq!(
            resolved.nack_disposition,
            PacketRecoveryDisposition::SkippedChainBroken
        );
        assert_eq!(
            resolved.frame_unrecoverable_reason,
            Some("sampleLossReferenceLowRepairability")
        );
    }

    #[test]
    fn oos_signal_lowers_non_keyframe_admission_priority() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.video_rtt_ms = Some(80.0);
        }
        source.recent_oos_active_until_ms = Some(220.0);

        let policy = sample_loss_nack_policy(
            91_400,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, true, 48 * 1024),
                false,
                Some(80.0),
                None,
                Some(260.0),
                false,
                FrameBudgetWindowSource::Transport,
            ),
            260.0,
            0.7,
            true,
            false,
            Some(80.0),
        );
        let base_priority = policy.priority;
        let resolved = source.with_cloud_latency_admission_policy(policy, 180.0, Some(0.7));

        assert_eq!(
            resolved.nack_disposition,
            PacketRecoveryDisposition::Attempted
        );
        assert!(resolved.priority < base_priority);
    }

    #[test]
    fn oos_signal_with_low_value_delta_is_skipped_but_reference_is_not() {
        // OOS + LowValue + delta（非 chain broken 状态）应被降级为 SkippedLowValue。
        // OOS + reference 帧不应被降级（frame_importance != "delta" 条件不满足）。
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.video_rtt_ms = Some(120.0); // >= 100ms 使 rtt_slack = Tight
        }
        // 激活 OOS 信号。
        source.recent_oos_active_until_ms = Some(220.0);

        // delta 帧（LowValue tier，非 chain broken）：OOS 应降级为 SkippedLowValue。
        // 需要 RTT >= 120ms 且 deadline - estimated_arrival <= 12ms 使 rtt_slack = Tight，
        // 从而 prefers_low_value_skip() 为 true，value_tier = LowValue。
        // estimated_arrival ≈ now_ms(180) + rtt*0.5(60) = 240，deadline=248 → slack=8 → Tight。
        let delta_policy = sample_loss_nack_policy(
            91_400,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, false, 12 * 1024),
                false, // 非 chain broken 状态
                Some(120.0),
                None,
                Some(248.0),
                false,
                FrameBudgetWindowSource::Transport,
            ),
            248.0,
            0.5,
            true,
            false,
            Some(120.0),
        );
        let delta_resolved =
            source.with_cloud_latency_admission_policy(delta_policy, 180.0, Some(0.5));
        assert_eq!(
            delta_resolved.nack_disposition,
            PacketRecoveryDisposition::SkippedLowValue
        );
        assert_eq!(
            delta_resolved.frame_unrecoverable_reason,
            Some("oosLowValueAdmission")
        );

        // reference 帧：OOS 路径的 frame_importance == "delta" 条件不满足，不应被降级。
        let ref_policy = sample_loss_nack_policy(
            91_401,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, true, 48 * 1024),
                false,
                Some(80.0),
                None,
                Some(260.0),
                false,
                FrameBudgetWindowSource::Transport,
            ),
            260.0,
            0.8,
            true,
            false,
            Some(80.0),
        );
        let ref_resolved = source.with_cloud_latency_admission_policy(ref_policy, 180.0, Some(0.8));
        assert_ne!(
            ref_resolved.nack_disposition,
            PacketRecoveryDisposition::SkippedLowValue
        );
        assert_ne!(
            ref_resolved.frame_unrecoverable_reason,
            Some("oosLowValueAdmission")
        );
    }

    #[test]
    fn display_starved_low_value_skip_stays_skipped_low_value_under_recovery_pressure() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 4,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repair-in-flight".to_string(),
                        sequence: Some(58),
                        frame_rtp_timestamp: None,
                        frame_importance: Some("supply".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: 150.0,
                    }),
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 150.0,
                    },
                    observed_at_ms: 150.0,
                });
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 48;
            stats.latest_video_host_present_time_ms = Some(0.0);
        }

        let policy = rtp_gap_nack_policy(
            FrameValue::new(false, false, 12 * 1024),
            FrameBudgetContext::for_transport(
                FrameValue::new(false, false, 12 * 1024),
                false,
                Some(40.0),
                None,
                Some(260.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
            260.0,
            true,
            false,
            Some(40.0),
        );
        let resolved = source.with_cloud_latency_admission_policy(policy, 450.0, None);

        assert_eq!(
            resolved.nack_disposition,
            PacketRecoveryDisposition::SkippedLowValue
        );
        assert_eq!(
            resolved.frame_unrecoverable_reason,
            Some("displayStarvedLowValueAdmission")
        );
    }

    #[test]
    fn startup_supply_near_deadline_keeps_nack_attempt() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("startup".to_string());
            stats.video_rtt_ms = Some(140.0);
        }

        let policy = sample_loss_nack_policy(
            91_280,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, true, 48 * 1024),
                false,
                Some(140.0),
                None,
                Some(254.0),
                true,
                FrameBudgetWindowSource::Transport,
            ),
            254.0,
            0.72,
            true,
            true,
            Some(140.0),
        );
        let resolved = source.with_cloud_latency_admission_policy(policy, 180.0, Some(0.72));

        assert_eq!(
            resolved.nack_disposition,
            PacketRecoveryDisposition::Attempted
        );
        assert_eq!(resolved.frame_unrecoverable_reason, None);
    }

    #[test]
    fn steady_supply_near_deadline_keeps_nack_attempt_like_startup() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("streaming".to_string());
            stats.video_rtt_ms = Some(140.0);
        }

        let policy = sample_loss_nack_policy(
            91_300,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, true, 48 * 1024),
                false,
                Some(140.0),
                None,
                Some(254.0),
                false,
                FrameBudgetWindowSource::Transport,
            ),
            254.0,
            0.72,
            true,
            false,
            Some(140.0),
        );
        let resolved = source.with_cloud_latency_admission_policy(policy, 180.0, Some(0.72));

        assert_eq!(
            resolved.nack_disposition,
            PacketRecoveryDisposition::Attempted
        );
        assert_eq!(resolved.frame_unrecoverable_reason, None);
    }

    #[test]
    fn recovery_supply_near_deadline_is_skipped_too_late_not_chain_broken() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
            stats.video_rtt_ms = Some(140.0);
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 5,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repair-in-flight".to_string(),
                        sequence: Some(61),
                        frame_rtp_timestamp: None,
                        frame_importance: Some("supply".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: 150.0,
                    }),
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 150.0,
                    },
                    observed_at_ms: 150.0,
                });
        }

        let policy = sample_loss_nack_policy(
            91_320,
            false,
            FrameBudgetContext::for_transport(
                FrameValue::new(false, true, 48 * 1024),
                false,
                Some(140.0),
                None,
                Some(254.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
            254.0,
            0.72,
            true,
            false,
            Some(140.0),
        );
        let resolved = source.with_cloud_latency_admission_policy(policy, 180.0, Some(0.72));

        assert_eq!(
            resolved.nack_disposition,
            PacketRecoveryDisposition::SkippedTooLate
        );
        assert_eq!(
            resolved.frame_unrecoverable_reason,
            Some("estimatedArrivalNearDeadlineSupplyRecovery")
        );
    }

    #[test]
    fn low_value_skip_under_recovery_pressure_reopens_chain_recovery() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repair-in-flight".to_string(),
                        sequence: Some(44),
                        frame_rtp_timestamp: None,
                        frame_importance: Some("supply".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: 120.0,
                    }),
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 120.0,
                    },
                    observed_at_ms: 120.0,
                });
        }

        source.maybe_handle_chain_broken(
            &crate::transport::rtc::stream::nack_scheduler::SkippedNackBatch {
                sequences: vec![44],
                source: "rtpGap",
                frame_rtp_timestamp: Some(91_200),
                frame_is_keyframe: Some(false),
                frame_importance: "disposable",
                deadline_at_ms: Some(160.0),
                estimated_recovery_arrival_ms: Some(170.0),
                frame_playout_deadline_at_ms: Some(160.0),
                nack_disposition: PacketRecoveryDisposition::SkippedLowValue,
                frame_unrecoverable_reason: Some("cloudHighRttLowValueAdmission"),
                budget_context: FrameBudgetContext::for_transport(
                    FrameValue::new(false, false, 12 * 1024),
                    false,
                    Some(160.0),
                    Some(170.0),
                    Some(160.0),
                    false,
                    FrameBudgetWindowSource::Recovery,
                ),
            },
            140.0,
            true,
        );

        assert!(source.is_blocking_non_keyframe_admission());
    }

    #[test]
    fn low_value_skip_does_not_trigger_recovery_without_timeline_chain_broken() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats.clone(),
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repair-in-flight".to_string(),
                        sequence: Some(44),
                        frame_rtp_timestamp: None,
                        frame_importance: Some("supply".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: 120.0,
                    }),
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 120.0,
                    },
                    observed_at_ms: 120.0,
                });
        }

        source.maybe_handle_chain_broken(
            &crate::transport::rtc::stream::nack_scheduler::SkippedNackBatch {
                sequences: vec![44],
                source: "rtpGap",
                frame_rtp_timestamp: Some(91_200),
                frame_is_keyframe: Some(false),
                frame_importance: "disposable",
                deadline_at_ms: Some(160.0),
                estimated_recovery_arrival_ms: Some(170.0),
                frame_playout_deadline_at_ms: Some(160.0),
                nack_disposition: PacketRecoveryDisposition::SkippedLowValue,
                frame_unrecoverable_reason: Some("cloudHighRttLowValueAdmission"),
                budget_context: FrameBudgetContext::for_transport(
                    FrameValue::new(false, false, 12 * 1024),
                    false,
                    Some(160.0),
                    Some(170.0),
                    Some(160.0),
                    false,
                    FrameBudgetWindowSource::Recovery,
                ),
            },
            140.0,
            false,
        );

        assert!(!source.is_blocking_non_keyframe_admission());
    }

    #[derive(Clone, Default)]
    struct CaptureRtcpPort {
        payloads: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl RtcRtcpSendPort for CaptureRtcpPort {
        fn send_rtcp(&self, payload: &[u8]) -> Result<(), String> {
            self.payloads
                .lock()
                .expect("payloads lock")
                .push(payload.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    async fn send_nack_batch_uses_real_media_and_sender_ssrc() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let (transport_observation_tx, _transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let capture = CaptureRtcpPort::default();
        let payloads = capture.payloads.clone();
        let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(capture);
        let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
        let mut source = RtcVideoFrameSource::new(
            rx,
            transport_observation_tx,
            rtcp_port,
            runtime_stats,
            16,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(200),
            NackSchedulerConfig {
                max_age_ms: 1_000,
                frame_deadline_ms: 120,
                burst_count: 2,
                retry_interval_ms: 20,
                max_retry_count: 3,
            },
        );
        drop(tx);
        source.current_media_ssrc = Some(0x4455_6677);
        source.local_rtcp_sender_ssrc = 0x1122_3344;

        let batch = NackBatch {
            sequences: vec![12, 13, 15],
            retry_count: 1,
            source: "sampleLoss",
            frame_rtp_timestamp: Some(0x0102_0304),
            frame_is_keyframe: Some(false),
            frame_importance: "disposable",
            deadline_at_ms: Some(1_000.0),
            estimated_recovery_arrival_ms: Some(950.0),
            frame_playout_deadline_at_ms: Some(1_020.0),
            nack_disposition: PacketRecoveryDisposition::Attempted,
            frame_unrecoverable_reason: None,
            budget_context: FrameBudgetContext::for_transport(
                FrameValue::new(false, false, 8 * 1024),
                false,
                Some(40.0),
                None,
                Some(1_020.0),
                false,
                FrameBudgetWindowSource::Transport,
            ),
        };

        source
            .send_nack_batch("sent", &batch, 1_000.0)
            .await
            .expect("send should succeed");

        let captured = payloads.lock().expect("payloads lock");
        assert_eq!(captured.len(), 1);
        let mut raw = Bytes::copy_from_slice(&captured[0]);
        let packets = rtc_rtcp::packet::unmarshal(&mut raw).expect("nack payload should parse");
        let nack = packets
            .into_iter()
            .find_map(|packet| {
                packet
                    .as_any()
                    .downcast_ref::<TransportLayerNack>()
                    .cloned()
            })
            .expect("expected transport layer nack");
        assert_eq!(nack.media_ssrc, 0x4455_6677);
        assert_eq!(nack.sender_ssrc, 0x1122_3344);
    }
}
