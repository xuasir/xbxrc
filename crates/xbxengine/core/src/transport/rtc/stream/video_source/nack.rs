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
    RtcVideoFrameSource, TransportLossObservation, TransportObservation,
};

const DISPLAY_STARVED_LOW_VALUE_PRESENT_STALE_MS: f64 = 400.0;
const DISPLAY_STARVED_LOW_VALUE_NO_PENDING_STREAK_MIN: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepairValueTier {
    Anchor,
    Supply,
    LowValue,
}

impl RtcVideoFrameSource {
    pub(super) async fn maybe_run_nack_maintenance(&mut self) {
        let now_ms = now_ms_f64();
        let pending_before = self.nack_scheduler.pending_count();
        let missing_sequences = self.nack_window.missing_seq_numbers(self.nack_skip_last_n);
        let frame_value = self.current_transport_frame_value();
        let cloud_mode = self.is_cloud_transport_profile();
        let startup_mode = self.is_cloud_startup_transport_profile();
        let cloud_rtt_ms = self.cloud_nack_rtt_ms();
        let base_budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.waiting_for_recovery_keyframe(),
            Some(cloud_rtt_ms),
            None,
            None,
            startup_mode,
            FrameBudgetWindowSource::Transport,
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
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-reorder-pending",
                Some(sequence),
                None,
                now_ms,
            );
        }
        let policy = self.with_cloud_latency_admission_policy(policy, now_ms);
        let (initial_batch, skipped_batch) = self
            .nack_scheduler
            .observe_missing_sequences_with_policy(&missing_sequences, now_ms, policy);
        if let Some(skipped) = skipped_batch.as_ref() {
            let chain_broken = self.timeline_state.mark_gap_expired(
                &skipped.sequences,
                now_ms,
                skipped.frame_rtp_timestamp,
                skipped.frame_importance,
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
                Some(match skipped.frame_unrecoverable_reason {
                    Some("referenceChainUnrecoverable") => {
                        XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
                    }
                    Some("cloudHighRttLowValueAdmission") => {
                        XbxEngineAnchorCandidateFailureReason::ChainBrokenCloudHighRttLowValueAdmission
                    }
                    Some("displayStarvedLowValueAdmission") => XbxEngineAnchorCandidateFailureReason::ChainBrokenDisplayStarvedLowValueAdmission,
                    Some("deadline") => XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline,
                    _ => XbxEngineAnchorCandidateFailureReason::Unknown,
                }),
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
            self.send_nack_batch("sent", &initial_batch, now_ms).await;
        }

        let poll_result = self.nack_scheduler.poll(now_ms);
        for expired_batch in poll_result.expired_batches {
            let chain_broken = self.timeline_state.mark_gap_expired(
                &expired_batch.sequences,
                now_ms,
                expired_batch.frame_rtp_timestamp,
                expired_batch.frame_importance,
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
            );
            if let Some(sequence) = retry_batch.sequences.first().copied() {
                self.record_video_timeline_observation(
                    "gap-repair-retry",
                    Some(sequence),
                    retry_batch.frame_rtp_timestamp,
                    now_ms,
                );
            }
            self.send_nack_batch("sent", &retry_batch, now_ms).await;
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
        let frame_value = self.current_transport_frame_value();
        let cloud_mode = self.is_cloud_transport_profile();
        let startup_mode = self.is_cloud_startup_transport_profile();
        let cloud_rtt_ms = self.cloud_nack_rtt_ms();
        let base_budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.waiting_for_recovery_keyframe(),
            Some(cloud_rtt_ms),
            None,
            None,
            startup_mode,
            FrameBudgetWindowSource::Transport,
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
        self.timeline_state
            .observe_gap(&missing_sequences, now_ms, None, policy.frame_importance);
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
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-nack-candidate",
                Some(sequence),
                None,
                now_ms,
            );
        }
        let policy = self.with_cloud_latency_admission_policy(policy, now_ms);
        let (initial_batch, skipped_batch) = self
            .nack_scheduler
            .observe_missing_sequences_with_policy(&missing_sequences, now_ms, policy);
        if let Some(skipped) = skipped_batch.as_ref() {
            let chain_broken = self.timeline_state.mark_gap_expired(
                &skipped.sequences,
                now_ms,
                skipped.frame_rtp_timestamp,
                skipped.frame_importance,
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
                Some(match skipped.frame_unrecoverable_reason {
                    Some("referenceChainUnrecoverable") => {
                        XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
                    }
                    Some("cloudHighRttLowValueAdmission") => {
                        XbxEngineAnchorCandidateFailureReason::ChainBrokenCloudHighRttLowValueAdmission
                    }
                    Some("displayStarvedLowValueAdmission") => XbxEngineAnchorCandidateFailureReason::ChainBrokenDisplayStarvedLowValueAdmission,
                    Some("deadline") => XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline,
                    _ => XbxEngineAnchorCandidateFailureReason::Unknown,
                }),
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
        self.send_nack_batch("sent", &initial_batch, now_ms).await;
    }

    pub(super) async fn send_nack_batch(&mut self, action: &str, batch: &NackBatch, now_ms: f64) {
        if batch.sequences.is_empty() {
            return;
        }
        let Some(media_ssrc) = self.current_media_ssrc else {
            crate::xbx_log_warn!(
                "[RtcVideoFrameSource] skip nack send action={} because media ssrc is unavailable",
                action
            );
            return;
        };

        let nack = TransportLayerNack {
            sender_ssrc: self.local_rtcp_sender_ssrc,
            media_ssrc,
            nacks: nack_pairs_from_sequence_numbers(&batch.sequences),
        };
        let mut buf = vec![0u8; nack.marshal_size()];
        if let Ok(_) = nack.marshal_to(&mut buf) {
            self.rtcp_port.send_rtcp(&buf);
        } else {
            crate::xbx_log_warn!(
                "[RtcVideoFrameSource] nack serialize failed action={}",
                action
            );
            return;
        }

        self.runtime_stats
            .record_nack_sent(batch.sequences.len(), self.nack_scheduler.pending_count());
        self.record_nack_observation(action, batch, now_ms);
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
        let mut missing_sequences =
            self.collect_missing_sequences_for_sample(sample_rtp_timestamp, media_dropped_packets);
        if missing_sequences.is_empty() {
            missing_sequences = self.collect_recent_missing_sequences(media_dropped_packets);
        }
        if missing_sequences.is_empty() {
            return false;
        }
        let frame_value = frame_value_for_importance(frame_importance);
        let repairability = self.estimate_repairability(
            frame_importance,
            media_dropped_packets,
            missing_sequences.len().min(u16::MAX as usize) as u16,
        );
        let base_deadline_at_ms = self
            .transport_deadline_tracker
            .next_transport_deadline_with_context_at_ms(
                now_ms,
                frame_value,
                FrameBudgetContext::for_transport(
                    frame_value,
                    self.waiting_for_recovery_keyframe(),
                    Some(self.cloud_nack_rtt_ms()),
                    None,
                    None,
                    self.is_cloud_startup_transport_profile(),
                    FrameBudgetWindowSource::Recovery,
                ),
            );
        let deadline_at_ms =
            self.dynamic_repair_deadline(now_ms, base_deadline_at_ms, repairability);
        let budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.waiting_for_recovery_keyframe(),
            Some(self.cloud_nack_rtt_ms()),
            None,
            Some(deadline_at_ms),
            self.is_cloud_startup_transport_profile(),
            FrameBudgetWindowSource::Recovery,
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
            frame_importance,
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
        );
        if let Some(sequence) = missing_sequences.first().copied() {
            self.record_video_timeline_observation(
                "gap-nack-candidate",
                Some(sequence),
                Some(sample_rtp_timestamp),
                now_ms,
            );
        }
        let policy = self.with_cloud_latency_admission_policy(policy, now_ms);
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
                Some(match skipped.frame_unrecoverable_reason {
                    Some("referenceChainUnrecoverable") => {
                        XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
                    }
                    Some("cloudHighRttLowValueAdmission") => {
                        XbxEngineAnchorCandidateFailureReason::ChainBrokenCloudHighRttLowValueAdmission
                    }
                    Some("displayStarvedLowValueAdmission") => XbxEngineAnchorCandidateFailureReason::ChainBrokenDisplayStarvedLowValueAdmission,
                    Some("deadline") => XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline,
                    _ => XbxEngineAnchorCandidateFailureReason::Unknown,
                }),
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
                "sampleLoss",
                Some(sample_rtp_timestamp),
                Some((missing_sequences.len() + 1).min(u16::MAX as usize) as u16),
                Some(media_dropped_packets),
                Some(frame_is_keyframe),
                Some(frame_importance),
            );
        }
        self.send_nack_batch("sent", &batch, now_ms).await;
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
        if frame_importance == "delta" {
            return Some(FrameRecoveryDisposition::UnrecoverableLate);
        }
        Some(FrameRecoveryDisposition::UnrecoverableReferenceChain)
    }

    fn with_cloud_latency_admission_policy(
        &self,
        mut policy: NackObservePolicy,
        now_ms: f64,
    ) -> NackObservePolicy {
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
        policy.budget_context = FrameBudgetContext::for_transport(
            frame_value,
            self.waiting_for_recovery_keyframe(),
            Some(self.cloud_nack_rtt_ms()),
            Some(estimated_recovery_arrival_ms),
            policy.deadline_at_ms,
            self.is_cloud_startup_transport_profile(),
            window_source_for_policy(policy.source),
        );
        if !policy.frame_is_keyframe.unwrap_or(false) {
            policy.frame_importance = policy.budget_context.frame_importance();
        }
        policy.priority = policy.budget_context.repair_priority(frame_value);

        let value_tier = classify_repair_value_tier(
            policy.budget_context,
            policy.frame_is_keyframe.unwrap_or(false),
            self.is_cloud_startup_transport_profile(),
        );

        if policy.budget_context.prefers_chain_broken()
            && !matches!(value_tier, RepairValueTier::Anchor)
        {
            policy.nack_disposition = PacketRecoveryDisposition::SkippedChainBroken;
            if policy.frame_unrecoverable_reason.is_none() {
                policy.frame_unrecoverable_reason = Some(
                    if matches!(value_tier, RepairValueTier::LowValue)
                        && policy.frame_importance == "delta"
                    {
                        "localBackpressureDeltaGap"
                    } else {
                        "awaitingRecoveryKeyframe"
                    },
                );
            }
            return policy;
        }

        if self.is_cloud_high_rtt_path()
            && matches!(value_tier, RepairValueTier::LowValue)
            && policy.budget_context.prefers_low_value_skip()
        {
            policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
            if policy.frame_unrecoverable_reason.is_none() {
                policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");
            }
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
        if frame_importance != "delta" || frame_is_keyframe.unwrap_or(false) {
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
        if skipped.frame_importance == "delta"
            && matches!(
                skipped.frame_unrecoverable_reason,
                Some(
                    "localBackpressureDeltaGap"
                        | "cloudHighRttLowValueAdmission"
                        | "displayStarvedLowValueAdmission"
                )
            )
        {
            return;
        }
        if skipped.nack_disposition != PacketRecoveryDisposition::SkippedChainBroken
            && !chain_broken
        {
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
        if !chain_broken || self.waiting_for_recovery_keyframe() {
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
        if let Some(flushed_batch) = self
            .nack_scheduler
            .flush_non_keyframe_pending("flushedAfterChainBrokenAdmission")
        {
            self.timeline_state.mark_gap_expired(
                &flushed_batch.sequences,
                now_ms,
                flushed_batch.frame_rtp_timestamp,
                flushed_batch.frame_importance,
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
        self.timeline_state.on_recovery_keyframe_requested();
        self.record_video_timeline_observation(
            "chain-recovery-keyframe-requested",
            None,
            frame_rtp_timestamp,
            now_ms,
        );
        self.set_waiting_for_recovery_keyframe(true);
        self.queue_transport_observation(TransportObservation::Loss(
            TransportLossObservation::RecoveryKeyframeRequested,
        ));
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

    fn current_transport_frame_value(&self) -> FrameValue {
        self.last_submitted_frame_value
    }

    fn estimate_repairability(
        &self,
        frame_importance: &'static str,
        media_dropped_packets: u16,
        missing_sequence_count: u16,
    ) -> f64 {
        let base = match frame_importance {
            "keyframe" => 0.95,
            "reference" => 0.8,
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
        let waiting_penalty = if self.waiting_for_recovery_keyframe() {
            0.06
        } else {
            0.0
        };
        (base + recovery_bonus - burst_penalty - late_penalty - missing_penalty - waiting_penalty)
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
            base_window_ms = base_window_ms.max(rtt_ms + cloud_nack_rtt_margin_ms(false, Some(rtt_ms)));
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

    fn is_cloud_startup_transport_profile(&self) -> bool {
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

    fn cloud_nack_rtt_ms(&self) -> f64 {
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
    use super::should_soften_display_starved_low_value_gap_from_runtime;

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
}

fn classify_repair_value_tier(
    budget_context: FrameBudgetContext,
    frame_is_keyframe: bool,
    cloud_startup_mode: bool,
) -> RepairValueTier {
    if frame_is_keyframe || matches!(budget_context.frame_importance(), "keyframe") {
        return RepairValueTier::Anchor;
    }
    if matches!(budget_context.frame_importance(), "reference") {
        return RepairValueTier::Supply;
    }
    if budget_context.prefers_low_value_skip() && !cloud_startup_mode {
        return RepairValueTier::LowValue;
    }
    RepairValueTier::Supply
}

fn window_source_for_policy(source: &'static str) -> FrameBudgetWindowSource {
    match source {
        "sampleLoss" => FrameBudgetWindowSource::Recovery,
        _ => FrameBudgetWindowSource::Transport,
    }
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
            RepairValueTier::LowValue
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
            RepairValueTier::Anchor
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
            RepairValueTier::Supply
        );
    }

    #[derive(Clone, Default)]
    struct CaptureRtcpPort {
        payloads: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl RtcRtcpSendPort for CaptureRtcpPort {
        fn send_rtcp(&self, payload: &[u8]) {
            self.payloads
                .lock()
                .expect("payloads lock")
                .push(payload.to_vec());
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
            frame_importance: "delta",
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

        source.send_nack_batch("sent", &batch, 1_000.0).await;

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
