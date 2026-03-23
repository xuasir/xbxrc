use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::transport::rtc::stream::nack_scheduler::{NackBatch, ResolvedNack};
use crate::XbxEngineVideoNackObservation;

use super::{
    capitalize_reason, nack_policy::*, now_ms_f64, FrameValue, RecentRtpPacket,
    RtcVideoFrameSource, TransportObservation,
};
use webrtc::util::{Marshal, MarshalSize};

impl RtcVideoFrameSource {
    pub(super) async fn maybe_run_nack_maintenance(&mut self) {
        let now_ms = now_ms_f64();
        let pending_before = self.nack_scheduler.pending_count();
        let missing_sequences = self.nack_window.missing_seq_numbers(self.nack_skip_last_n);
        let frame_value = self.current_transport_frame_value();
        let cloud_mode = self.is_cloud_transport_profile();
        let startup_mode = self.is_cloud_startup_transport_profile();
        let cloud_rtt_ms = self.cloud_nack_rtt_ms();
        let deadline_at_ms = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            self.transport_deadline_tracker
                .next_transport_deadline_for_value_at_ms(now_ms, frame_value),
            cloud_mode,
            startup_mode,
            Some(cloud_rtt_ms),
        );
        if let Some(initial_batch) = self.nack_scheduler.observe_missing_sequences_with_policy(
            &missing_sequences,
            now_ms,
            rtp_window_nack_policy(
                frame_value,
                deadline_at_ms,
                cloud_mode,
                startup_mode,
                Some(cloud_rtt_ms),
            ),
        ) {
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
                },
                now_ms,
            );
        }
        if let Some(retry_batch) = poll_result.retry_batch {
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
        let deadline_at_ms = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            self.transport_deadline_tracker
                .next_transport_deadline_for_value_at_ms(now_ms, frame_value),
            cloud_mode,
            startup_mode,
            Some(cloud_rtt_ms),
        );
        let Some(initial_batch) = self.nack_scheduler.observe_missing_sequences_with_policy(
            &missing_sequences,
            now_ms,
            rtp_gap_nack_policy(
                frame_value,
                deadline_at_ms,
                cloud_mode,
                startup_mode,
                Some(cloud_rtt_ms),
            ),
        ) else {
            return;
        };
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

        let nack = webrtc::rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack {
            sender_ssrc: 0,
            media_ssrc: 0,
            nacks: webrtc::rtcp::transport_feedbacks::transport_layer_nack::nack_pairs_from_sequence_numbers(&batch.sequences),
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
                observed_at_ms: now_ms,
            });
    }

    pub(super) fn record_nack_recovered(&mut self, resolved: ResolvedNack, now_ms: f64) {
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
            .next_transport_deadline_for_value_at_ms(now_ms, frame_value);
        let deadline_at_ms =
            self.dynamic_repair_deadline(now_ms, base_deadline_at_ms, repairability);
        let policy = sample_loss_nack_policy(
            sample_rtp_timestamp,
            frame_is_keyframe,
            frame_importance,
            deadline_at_ms,
            repairability,
            self.is_cloud_transport_profile(),
            self.is_cloud_startup_transport_profile(),
            Some(self.cloud_nack_rtt_ms()),
        );
        let pending_before = self.nack_scheduler.pending_count();
        let Some(batch) = self.nack_scheduler.observe_missing_sequences_with_policy(
            &missing_sequences,
            now_ms,
            policy,
        ) else {
            return false;
        };
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
        if self.waiting_for_recovery_keyframe {
            FrameValue::new(true, true, 96 * 1024)
        } else {
            self.last_submitted_frame_value
        }
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
        let waiting_penalty = if self.waiting_for_recovery_keyframe {
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
                .max(rtt_ms + CLOUD_STARTUP_NACK_RTT_MARGIN_MS)
                .max(CLOUD_STARTUP_HEAD_HOLE_DEADLINE_FLOOR_MS);
            (1.05 + repairability * 0.7).clamp(1.05, 1.7)
        } else if self.is_cloud_transport_profile() {
            // cloud 场景直接按运行时 RTT 放宽恢复窗口，不再额外加硬下限。
            let rtt_ms = self.cloud_nack_rtt_ms();
            base_window_ms = base_window_ms.max(rtt_ms + CLOUD_NACK_RTT_MARGIN_MS);
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
fn wrapping_sequence_range(start: u16, end_exclusive: u16) -> Vec<u16> {
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
}
