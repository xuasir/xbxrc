use crate::transport::webrtc::nack_scheduler::{NackBatch, NackObservePolicy, ResolvedNack};
use crate::XbxEngineVideoNackObservation;

use super::{
    capitalize_reason, now_ms_f64, FrameValue, NackSequenceWindow, RecentRtpPacket,
    VideoRecoverySignal, WebrtcVideoAdapter, UINT16SIZE_HALF,
};

impl WebrtcVideoAdapter {
    pub(super) async fn maybe_run_nack_maintenance(&mut self) {
        let now_ms = now_ms_f64();
        let pending_before = self.nack_scheduler.pending_count();
        let deadline_at_ms = self
            .frame_deadline_tracker
            .next_deadline_for_value_at_ms(now_ms, FrameValue::new(false, false, 0));
        let missing_sequences = self.nack_window.missing_seq_numbers(self.nack_skip_last_n);
        if let Some(initial_batch) = self.nack_scheduler.observe_missing_sequences(
            &missing_sequences,
            now_ms,
            Some(deadline_at_ms),
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
                if let Ok(mut stats) = self.runtime_stats.lock() {
                    stats.inbound_video_packet_loss_estimate_total = stats
                        .inbound_video_packet_loss_estimate_total
                        .saturating_add(u64::from(inserted_count));
                }
            }
            self.send_nack_batch("sent", &initial_batch, now_ms).await;
        }

        let poll_result = self.nack_scheduler.poll(now_ms);
        for expired_batch in poll_result.expired_batches {
            if expired_batch.reason == "deadline" {
                let is_severe_gap =
                    expired_batch.sequences.len() >= self.severe_deadline_packet_threshold;
                self.queue_recovery_signal(if is_severe_gap {
                    VideoRecoverySignal::TransportSevereDeadline
                } else {
                    VideoRecoverySignal::TransportExpiredDeadline
                });
            }
            if let Ok(mut stats) = self.runtime_stats.lock() {
                stats.video_loss_finalized_count_total = stats
                    .video_loss_finalized_count_total
                    .saturating_add(expired_batch.sequences.len() as u64);
            }
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
        if let Ok(mut stats) = self.runtime_stats.lock() {
            stats.video_pending_missing_packets = self.nack_scheduler.pending_count();
        }
    }

    pub(super) async fn observe_forward_gap_and_nack(
        &mut self,
        expected_sequence: u16,
        received_sequence: u16,
    ) {
        let now_ms = now_ms_f64();
        let pending_before = self.nack_scheduler.pending_count();
        let deadline_at_ms = self
            .frame_deadline_tracker
            .next_deadline_for_value_at_ms(now_ms, FrameValue::new(false, false, 0));
        let Some(initial_batch) = self.nack_scheduler.observe_gap(
            expected_sequence,
            received_sequence,
            now_ms,
            Some(deadline_at_ms),
        ) else {
            return;
        };
        let inserted_count = self
            .nack_scheduler
            .pending_count()
            .saturating_sub(pending_before)
            .min(u16::MAX as usize) as u16;
        if inserted_count > 0 {
            let missing_sequences = wrapping_sequence_range(expected_sequence, received_sequence);
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
            if let Ok(mut stats) = self.runtime_stats.lock() {
                stats.inbound_video_packet_loss_estimate_total = stats
                    .inbound_video_packet_loss_estimate_total
                    .saturating_add(u64::from(inserted_count));
            }
        }
        self.send_nack_batch("sent", &initial_batch, now_ms).await;
    }

    pub(super) async fn send_nack_batch(&mut self, action: &str, batch: &NackBatch, now_ms: f64) {
        if batch.sequences.is_empty() {
            return;
        }
        use webrtc::rtcp::transport_feedbacks::transport_layer_nack::{
            nack_pairs_from_sequence_numbers, TransportLayerNack,
        };

        let nack = TransportLayerNack {
            sender_ssrc: 0,
            media_ssrc: self.track.ssrc(),
            nacks: nack_pairs_from_sequence_numbers(&batch.sequences),
        };

        if let Err(error) = self.peer_connection.write_rtcp(&[Box::new(nack)]).await {
            crate::xbx_log_warn!(
                "[WebrtcVideoAdapter] nack send failed action={} err={}",
                action,
                error
            );
            return;
        }

        if let Ok(mut stats) = self.runtime_stats.lock() {
            stats.video_nack_batch_count_total =
                stats.video_nack_batch_count_total.saturating_add(1);
            stats.video_nack_request_count_total = stats
                .video_nack_request_count_total
                .saturating_add(batch.sequences.len() as u64);
            stats.video_pending_missing_packets = self.nack_scheduler.pending_count();
        }
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
        if let Ok(mut stats) = self.runtime_stats.lock() {
            stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
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
    }

    pub(super) fn record_nack_recovered(&mut self, resolved: ResolvedNack, now_ms: f64) {
        self.nack_observation_id = self.nack_observation_id.saturating_add(1);
        if let Ok(mut stats) = self.runtime_stats.lock() {
            stats.video_pending_missing_packets = self.nack_scheduler.pending_count();
            if resolved.was_late {
                stats.video_loss_late_recovered_count_total = stats
                    .video_loss_late_recovered_count_total
                    .saturating_add(1);
            } else {
                stats.video_loss_recovered_count_total =
                    stats.video_loss_recovered_count_total.saturating_add(1);
            }
            stats.video_nack_recovery_rtt_ms = Some(resolved.recovery_time_ms);
            stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
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
            });
        }
        if resolved.was_late {
            self.queue_recovery_signal(VideoRecoverySignal::TransportRecoveredLate);
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
        if let Ok(mut stats) = self.runtime_stats.lock() {
            stats.latest_video_packet_gap = Some(crate::XbxEngineVideoPacketGapObservation {
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
            });
            stats.latest_video_packet_sequence = Some(last_sequence);
        }
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
        let frame_value = match frame_importance {
            "keyframe" => FrameValue::new(true, false, 128 * 1024),
            "reference" => FrameValue::new(false, true, 48 * 1024),
            _ => FrameValue::new(false, false, 12 * 1024),
        };
        let deadline_at_ms = self
            .frame_deadline_tracker
            .next_deadline_for_value_at_ms(now_ms, frame_value);
        let policy = sample_loss_nack_policy(
            sample_rtp_timestamp,
            frame_is_keyframe,
            frame_importance,
            deadline_at_ms,
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
        if missing.len() > desired {
            missing = missing[missing.len().saturating_sub(desired)..].to_vec();
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
        if missing.len() > desired {
            missing = missing[missing.len().saturating_sub(desired)..].to_vec();
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
}

fn sample_loss_nack_policy(
    sample_rtp_timestamp: u32,
    frame_is_keyframe: bool,
    frame_importance: &'static str,
    deadline_at_ms: f64,
) -> NackObservePolicy {
    let (max_age_ms, retry_interval_ms, burst_count, priority) = match frame_importance {
        "keyframe" => (28, 10, 4, 3),
        "reference" => (18, 8, 3, 2),
        _ => (12, 6, 2, 1),
    };
    NackObservePolicy {
        source: "sampleLoss",
        deadline_at_ms: Some(deadline_at_ms),
        max_age_ms: Some(max_age_ms),
        retry_interval_ms: Some(retry_interval_ms),
        burst_count: Some(burst_count),
        frame_rtp_timestamp: Some(sample_rtp_timestamp),
        frame_is_keyframe: Some(frame_is_keyframe),
        frame_importance,
        priority,
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

impl NackSequenceWindow {
    pub(super) fn new(log2_size_minus_6: u8) -> Self {
        Self {
            packets: vec![0u64; 1 << log2_size_minus_6],
            size: 1 << (log2_size_minus_6 + 6),
            end: 0,
            started: false,
            last_consecutive: 0,
        }
    }

    // 直接沿用 webrtc 默认 generator 的环形接收窗口语义，避免我们再发明一套缺包判定。
    pub(super) fn add(&mut self, seq: u16) {
        if !self.started {
            self.set_received(seq);
            self.end = seq;
            self.started = true;
            self.last_consecutive = seq;
            return;
        }

        let last_consecutive_plus1 = self.last_consecutive.wrapping_add(1);
        let diff = seq.wrapping_sub(self.end);
        if diff == 0 {
            return;
        } else if diff < UINT16SIZE_HALF {
            let mut i = self.end.wrapping_add(1);
            while i != seq {
                self.del_received(i);
                i = i.wrapping_add(1);
            }
            self.end = seq;

            let seq_sub_last_consecutive = seq.wrapping_sub(self.last_consecutive);
            if last_consecutive_plus1 == seq {
                self.last_consecutive = seq;
            } else if seq_sub_last_consecutive > self.size {
                let diff = seq.wrapping_sub(self.size);
                self.last_consecutive = diff;
                self.fix_last_consecutive();
            }
        } else if last_consecutive_plus1 == seq {
            self.last_consecutive = seq;
            self.fix_last_consecutive();
        }

        self.set_received(seq);
    }

    pub(super) fn missing_seq_numbers(&self, skip_last_n: u16) -> Vec<u16> {
        let until = self.end.wrapping_sub(skip_last_n);
        let diff = until.wrapping_sub(self.last_consecutive);
        if diff >= UINT16SIZE_HALF {
            return vec![];
        }

        let mut missing = vec![];
        let mut i = self.last_consecutive.wrapping_add(1);
        let until_plus_1 = until.wrapping_add(1);
        while i != until_plus_1 {
            if !self.get_received(i) {
                missing.push(i);
            }
            i = i.wrapping_add(1);
        }
        missing
    }

    pub(super) fn missing_seq_numbers_in_range(&self, start: u16, end_exclusive: u16) -> Vec<u16> {
        let diff = end_exclusive.wrapping_sub(start);
        if diff == 0 || diff >= UINT16SIZE_HALF {
            return vec![];
        }

        let mut missing = Vec::new();
        let mut cursor = start;
        while cursor != end_exclusive {
            if !self.get_received(cursor) {
                missing.push(cursor);
            }
            cursor = cursor.wrapping_add(1);
        }
        missing
    }

    fn set_received(&mut self, seq: u16) {
        let pos = (seq % self.size) as usize;
        self.packets[pos / 64] |= 1u64 << (pos % 64);
    }

    fn del_received(&mut self, seq: u16) {
        let pos = (seq % self.size) as usize;
        self.packets[pos / 64] &= u64::MAX ^ (1u64 << (pos % 64));
    }

    fn get_received(&self, seq: u16) -> bool {
        let pos = (seq % self.size) as usize;
        (self.packets[pos / 64] & (1u64 << (pos % 64))) != 0
    }

    fn fix_last_consecutive(&mut self) {
        let mut i = self.last_consecutive.wrapping_add(1);
        while i != self.end.wrapping_add(1) && self.get_received(i) {
            i = i.wrapping_add(1);
        }
        self.last_consecutive = i.wrapping_sub(1);
    }
}
