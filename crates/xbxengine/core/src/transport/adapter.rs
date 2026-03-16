use bytes::Bytes;
use rtp::codecs::h264::H264Packet;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_remote::TrackRemote;
use webrtc_media::io::sample_builder::SampleBuilder;

use crate::media::video::types::{EncodedFrame, FrameValue, VideoCodec};
use crate::transport::h264_resolution::parse_sps_dimensions_from_nal;
use crate::transport::webrtc::escalation::VideoEscalationReason;
use crate::transport::webrtc::frame_deadline::FrameDeadlineTracker;
use crate::transport::webrtc::nack_scheduler::{
    NackObservePolicy, NackScheduler, NackSchedulerConfig, ResolvedNack,
};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoNackObservation};

pub enum FrameSourceEvent {
    Frame(EncodedFrame),
    EscalationHint {
        reason: VideoEscalationReason,
        label: &'static str,
    },
}

pub trait FrameSource: Send {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<FrameSourceEvent>> + Send + 'a>>;
}

pub struct WebrtcVideoAdapter {
    track: Arc<TrackRemote>,
    peer_connection: Arc<RTCPeerConnection>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    sample_builder: SampleBuilder<H264Packet>,
    max_late_packets: u16,
    jitter_buffer_min_delay: Duration,
    jitter_buffer_max_delay: Duration,
    idle_timeout: std::time::Duration,
    idle_hint_cooldown: std::time::Duration,
    last_packet_time: std::time::Instant,
    assembling_frame_start: Option<std::time::Instant>,
    current_assembly_packet_count: u16,
    last_idle_hint_time: Option<std::time::Instant>,
    assembly_stall_timeout: std::time::Duration,
    thin_stream_packet_threshold: u16,
    nack_scheduler: NackScheduler,
    nack_window: NackSequenceWindow,
    nack_skip_last_n: u16,
    last_highest_rtp_sequence: Option<u16>,
    recent_rtp_packets: VecDeque<RecentRtpPacket>,
    packet_gap_observation_id: u64,
    frame_deadline_tracker: FrameDeadlineTracker,
    nack_observation_id: u64,
    pending_escalation_hint: Option<(VideoEscalationReason, &'static str)>,
    severe_deadline_packet_threshold: usize,
    waiting_for_recovery_keyframe: bool,
    sample_loss_burst_count: u8,
    clean_samples_since_loss: u8,
    current_width: u32,
    current_height: u32,
}

impl WebrtcVideoAdapter {
    pub fn new(
        track: Arc<TrackRemote>,
        peer_connection: Arc<RTCPeerConnection>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        max_late_packets: u16,
        jitter_buffer_min_delay: Duration,
        jitter_buffer_max_delay: Duration,
        idle_timeout: std::time::Duration,
        nack_config: NackSchedulerConfig,
    ) -> Self {
        let frame_deadline_ms = nack_config.frame_deadline_ms;
        let burst_count = usize::from(nack_config.burst_count.max(1));
        let jitter_buffer_max_delay = jitter_buffer_max_delay.max(jitter_buffer_min_delay);
        let assembly_stall_timeout = idle_timeout
            .mul_f32(3.0)
            .clamp(Duration::from_millis(240), Duration::from_millis(600));
        Self {
            track,
            peer_connection,
            runtime_stats,
            sample_builder: build_sample_builder(max_late_packets, jitter_buffer_max_delay),
            max_late_packets,
            jitter_buffer_min_delay,
            jitter_buffer_max_delay,
            idle_timeout,
            idle_hint_cooldown: idle_timeout.max(std::time::Duration::from_millis(400)),
            last_packet_time: std::time::Instant::now(),
            assembling_frame_start: None,
            current_assembly_packet_count: 0,
            last_idle_hint_time: None,
            assembly_stall_timeout,
            thin_stream_packet_threshold: nack_config.burst_count.saturating_mul(6).max(18),
            nack_scheduler: NackScheduler::new(nack_config),
            nack_window: NackSequenceWindow::new(13 - 6),
            nack_skip_last_n: 2,
            last_highest_rtp_sequence: None,
            recent_rtp_packets: VecDeque::with_capacity(512),
            packet_gap_observation_id: 0,
            frame_deadline_tracker: FrameDeadlineTracker::new(frame_deadline_ms),
            nack_observation_id: 0,
            pending_escalation_hint: None,
            // 大范围 deadline 失效通常不是“再试一次 keyframe”能解决的，
            // 这里提前标成 severe，交给统一 escalation ladder 处理。
            severe_deadline_packet_threshold: (burst_count * 32).max(128),
            waiting_for_recovery_keyframe: false,
            sample_loss_burst_count: 0,
            clean_samples_since_loss: 0,
            current_width: 0,
            current_height: 0,
        }
    }

    async fn maybe_run_nack_maintenance(&mut self) {
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
                self.queue_escalation_hint(
                    if is_severe_gap {
                        VideoEscalationReason::TransportSevereDeadline
                    } else {
                        VideoEscalationReason::TransportExpiredDeadline
                    },
                    if is_severe_gap {
                        "transportSevereDeadline"
                    } else {
                        "transportExpiredDeadline"
                    },
                );
            }
            if let Ok(mut stats) = self.runtime_stats.lock() {
                stats.video_loss_finalized_count_total = stats
                    .video_loss_finalized_count_total
                    .saturating_add(expired_batch.sequences.len() as u64);
            }
            self.record_nack_observation(
                &format!("expired{}", capitalize_reason(&expired_batch.reason)),
                &crate::transport::webrtc::nack_scheduler::NackBatch {
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

    async fn observe_forward_gap_and_nack(
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

    async fn send_nack_batch(
        &mut self,
        action: &str,
        batch: &crate::transport::webrtc::nack_scheduler::NackBatch,
        now_ms: f64,
    ) {
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

    fn record_nack_observation(
        &mut self,
        action: &str,
        batch: &crate::transport::webrtc::nack_scheduler::NackBatch,
        now_ms: f64,
    ) {
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

    fn record_nack_recovered(&mut self, resolved: ResolvedNack, now_ms: f64) {
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
            self.queue_escalation_hint(
                VideoEscalationReason::TransportRecoveredLate,
                "transportRecoveredLate",
            );
        }
    }

    fn record_packet_gap_observation(
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

    async fn observe_sample_loss_and_nack(
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

    fn push_recent_rtp_packet(&mut self, sequence: u16, rtp_timestamp: u32) {
        if self.recent_rtp_packets.len() >= 512 {
            self.recent_rtp_packets.pop_front();
        }
        self.recent_rtp_packets.push_back(RecentRtpPacket {
            sequence,
            rtp_timestamp,
        });
    }

    fn should_trigger_thin_stream_stall(&self, now: std::time::Instant) -> bool {
        self.assembling_frame_start.is_some_and(|started_at| {
            now.duration_since(started_at) >= self.assembly_stall_timeout
                && self.current_assembly_packet_count > 0
                && self.current_assembly_packet_count <= self.thin_stream_packet_threshold
        })
    }

    fn queue_escalation_hint(&mut self, reason: VideoEscalationReason, label: &'static str) {
        if self.pending_escalation_hint.is_none() {
            self.pending_escalation_hint = Some((reason, label));
        }
    }
}

#[derive(Clone, Copy)]
struct RecentRtpPacket {
    sequence: u16,
    rtp_timestamp: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryKeyframeAction {
    Submit,
    DropAndRequestKeyframe,
    TriggerWaitKeyframe,
    WaitKeyframe,
}

fn resolve_recovery_keyframe_action(
    waiting_for_recovery_keyframe: bool,
    sample_loss_burst_count: u8,
    media_dropped_packets: u16,
    is_keyframe: bool,
) -> (bool, RecoveryKeyframeAction) {
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
        return (true, RecoveryKeyframeAction::WaitKeyframe);
    }

    (false, RecoveryKeyframeAction::Submit)
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

fn detect_forward_gap(
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

fn wrapping_sequence_range(start: u16, end_exclusive: u16) -> Vec<u16> {
    let mut sequences = Vec::new();
    let mut cursor = start;
    while cursor != end_exclusive {
        sequences.push(cursor);
        cursor = cursor.wrapping_add(1);
    }
    sequences
}

fn parse_idr_and_sps(payload: &[u8]) -> (bool, Option<(u32, u32)>) {
    let mut is_keyframe = false;
    let mut resolution = None;
    let mut i = 0;
    while i + 3 < payload.len() {
        let start_len = if payload[i] == 0 && payload[i + 1] == 0 && payload[i + 2] == 1 {
            3
        } else if i + 4 < payload.len()
            && payload[i] == 0
            && payload[i + 1] == 0
            && payload[i + 2] == 0
            && payload[i + 3] == 1
        {
            4
        } else {
            i += 1;
            continue;
        };

        if i + start_len >= payload.len() {
            break;
        }

        let nal_type = payload[i + start_len] & 0x1f;

        let mut nal_end = payload.len();
        let mut j = i + start_len;
        while j + 3 < payload.len() {
            if (payload[j] == 0 && payload[j + 1] == 0 && payload[j + 2] == 1)
                || (j + 4 < payload.len()
                    && payload[j] == 0
                    && payload[j + 1] == 0
                    && payload[j + 2] == 0
                    && payload[j + 3] == 1)
            {
                nal_end = j;
                break;
            }
            j += 1;
        }

        let nal = &payload[i + start_len..nal_end];
        if nal_type == 5 {
            is_keyframe = true;
        } else if nal_type == 7 {
            resolution = parse_sps_dimensions_from_nal(nal);
        }

        i = nal_end;
    }
    (is_keyframe, resolution)
}

impl FrameSource for WebrtcVideoAdapter {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<FrameSourceEvent>> + Send + 'a>>
    {
        Box::pin(async {
            loop {
                self.maybe_run_nack_maintenance().await;
                if let Some((reason, label)) = self.pending_escalation_hint.take() {
                    return Some(FrameSourceEvent::EscalationHint { reason, label });
                }
                if let Some(sample) = self.sample_builder.pop() {
                    self.last_packet_time = std::time::Instant::now();
                    self.assembling_frame_start = None;
                    self.current_assembly_packet_count = 0;
                    let payload = sample.data.to_vec();
                    let (is_keyframe, maybe_res) = parse_idr_and_sps(&payload);
                    let media_dropped_packets = sample
                        .prev_dropped_packets
                        .saturating_sub(sample.prev_padding_packets);
                    if is_keyframe {
                        self.sample_loss_burst_count = 0;
                        self.clean_samples_since_loss = 0;
                    } else if media_dropped_packets > 0 {
                        self.sample_loss_burst_count =
                            self.sample_loss_burst_count.saturating_add(1);
                        self.clean_samples_since_loss = 0;
                    } else if self.sample_loss_burst_count > 0 {
                        self.clean_samples_since_loss =
                            self.clean_samples_since_loss.saturating_add(1);
                        if self.clean_samples_since_loss >= 4 {
                            self.sample_loss_burst_count = 0;
                            self.clean_samples_since_loss = 0;
                        }
                    }
                    let (next_waiting_for_recovery_keyframe, recovery_action) =
                        resolve_recovery_keyframe_action(
                            self.waiting_for_recovery_keyframe,
                            self.sample_loss_burst_count,
                            media_dropped_packets,
                            is_keyframe,
                        );
                    self.waiting_for_recovery_keyframe = next_waiting_for_recovery_keyframe;

                    if media_dropped_packets > 0 {
                        if let Ok(mut stats) = self.runtime_stats.lock() {
                            stats.inbound_video_packet_loss_estimate_total = stats
                                .inbound_video_packet_loss_estimate_total
                                .saturating_add(u64::from(media_dropped_packets));
                        }
                        crate::xbx_log_warn!(
                            "[WebrtcVideoAdapter] media loss detected before sample ts={} dropped_packets={} is_keyframe={}",
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
                            // 单次样本丢包优先尝试低延迟 NACK，不为了追回旧帧长期阻塞视频链。
                            // 只有当前拿不到明确缺包序号时，才退回 keyframe 恢复。
                            let nack_started = self
                                .observe_sample_loss_and_nack(
                                    sample.packet_timestamp,
                                    media_dropped_packets,
                                    is_keyframe,
                                    sample_loss_frame_importance,
                                )
                                .await;
                            if !nack_started {
                                self.queue_escalation_hint(
                                    VideoEscalationReason::TransportSampleLoss,
                                    "transportSampleLoss",
                                );
                            }
                            continue;
                        }
                        RecoveryKeyframeAction::TriggerWaitKeyframe => {
                            // 连续样本丢包已经说明参考链大概率持续污染，
                            // 这里再升级到 wait-keyframe，避免坏帧长时间扩散。
                            self.queue_escalation_hint(
                                VideoEscalationReason::WaitKeyframe,
                                "transportSampleLoss",
                            );
                            continue;
                        }
                        RecoveryKeyframeAction::WaitKeyframe => {
                            self.queue_escalation_hint(
                                VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                                "transportAwaitRecoveryKeyframe",
                            );
                            continue;
                        }
                    }

                    let mut config_changed = false;
                    if let Some((w, h)) = maybe_res {
                        if w != self.current_width || h != self.current_height {
                            self.current_width = w;
                            self.current_height = h;
                            config_changed = true;
                        }
                    }

                    let frame_value = FrameValue::new(is_keyframe, config_changed, payload.len());
                    let playout_delay = resolve_playout_delay(
                        frame_value,
                        self.jitter_buffer_min_delay,
                        self.jitter_buffer_max_delay,
                    );
                    let target_playout_at_ms = now_ms_f64() + playout_delay.as_millis() as f64;
                    self.frame_deadline_tracker
                        .record_frame_target(target_playout_at_ms);

                    // --- 网络层健康度指标：有效 NALU 大小与分辨率 ---
                    crate::xbx_log_debug!(
                        "[Ingress] NALU Assb OK: size={}B, res={}x{}, is_kf={}",
                        payload.len(),
                        self.current_width,
                        self.current_height,
                        is_keyframe
                    );

                    return Some(FrameSourceEvent::Frame(EncodedFrame {
                        codec: VideoCodec::H264,
                        is_keyframe,
                        config_changed,
                        value: frame_value,
                        width: self.current_width,
                        height: self.current_height,
                        rtp_timestamp: sample.packet_timestamp,
                        assembled_at: std::time::Instant::now(),
                        target_playout_time: std::time::Instant::now() + playout_delay,
                        payload: Bytes::from(payload),
                    }));
                }

                let now = std::time::Instant::now();
                // 仅在网络真正停止传包时熔断
                // 注意：不使用帧装配超时（assembly_timeout），因为大 IDR 帧在 15 Mbps
                // 下可能需要 80-200ms 才能完整传输，短超时会反复打断装配形成死循环
                let idle_timeout = now.duration_since(self.last_packet_time) > self.idle_timeout;
                let thin_stream_stall = self.should_trigger_thin_stream_stall(now);

                if idle_timeout || thin_stream_stall {
                    self.sample_builder =
                        build_sample_builder(self.max_late_packets, self.jitter_buffer_max_delay);
                    self.assembling_frame_start = None;
                    self.current_assembly_packet_count = 0;
                    self.last_packet_time = now;

                    if self
                        .last_idle_hint_time
                        .map_or(true, |t| now.duration_since(t) >= self.idle_hint_cooldown)
                    {
                        self.last_idle_hint_time = Some(now);
                        return Some(FrameSourceEvent::EscalationHint {
                            reason: if thin_stream_stall {
                                VideoEscalationReason::AdapterThinStream
                            } else {
                                VideoEscalationReason::AdapterIdleTimeout
                            },
                            label: if thin_stream_stall {
                                "adapterThinStream"
                            } else {
                                "adapterIdleTimeout"
                            },
                        });
                    }
                    continue;
                }

                // 每次最多等 50ms，避免 pop 检查滞后太久
                let wait_duration = std::time::Duration::from_millis(50);
                match tokio::time::timeout(wait_duration, self.track.read_rtp()).await {
                    Ok(Ok((rtp, _))) => {
                        self.last_packet_time = std::time::Instant::now();
                        if self.assembling_frame_start.is_none() {
                            self.assembling_frame_start = Some(self.last_packet_time);
                            self.current_assembly_packet_count = 0;
                        }
                        self.current_assembly_packet_count =
                            self.current_assembly_packet_count.saturating_add(1);
                        let seq = rtp.header.sequence_number;
                        let now_ms = now_ms_f64();
                        let (next_highest_sequence, forward_gap) =
                            detect_forward_gap(self.last_highest_rtp_sequence, seq);
                        self.last_highest_rtp_sequence = next_highest_sequence;
                        if let Some((expected_sequence, received_sequence)) = forward_gap {
                            self.observe_forward_gap_and_nack(expected_sequence, received_sequence)
                                .await;
                        }
                        self.nack_window.add(seq);
                        self.push_recent_rtp_packet(seq, rtp.header.timestamp);
                        if let Some(resolved) = self.nack_scheduler.resolve_sequence(seq, now_ms) {
                            self.record_nack_recovered(resolved, now_ms);
                        }
                        if seq % 100 == 0 {
                            crate::xbx_log_info!(
                                "[WebrtcVideoAdapter] RTP packet received: seq={}, ts={}",
                                seq,
                                rtp.header.timestamp
                            );
                        }
                        self.sample_builder.push(rtp);
                    }
                    Ok(Err(e)) => {
                        if !e.to_string().contains("io: EOF") {
                            crate::xbx_log_error!("[WebrtcVideoAdapter] track read error: {}", e);
                        }
                        return None;
                    }
                    Err(_) => {
                        // Timeout hit: the start of the next loop will trigger `assembly_timeout` or `idle_timeout` and reset.
                    }
                }
            }
        })
    }
}

fn build_sample_builder(
    max_late_packets: u16,
    max_time_delay: Duration,
) -> SampleBuilder<H264Packet> {
    SampleBuilder::new(max_late_packets, H264Packet::default(), 90_000)
        .with_max_time_delay(max_time_delay)
}

fn resolve_playout_delay(value: FrameValue, min_delay: Duration, max_delay: Duration) -> Duration {
    if value.is_sync_point() || value.refresh_boost {
        return max_delay;
    }

    // delta 帧按价值比例在 min/max 之间插值，尽量降低 steady-state 排队时延。
    let ratio = value.deadline_budget_ratio_per_mille() as u128;
    let min_ms = min_delay.as_millis();
    let max_ms = max_delay.as_millis().max(min_ms);
    let spread_ms = max_ms.saturating_sub(min_ms);
    let scaled_ms = min_ms + (spread_ms * ratio / 1_000);
    Duration::from_millis(scaled_ms as u64).max(min_delay)
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn capitalize_reason(reason: &str) -> String {
    let mut chars = reason.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

const UINT16SIZE_HALF: u16 = 1 << 15;

struct NackSequenceWindow {
    packets: Vec<u64>,
    size: u16,
    end: u16,
    started: bool,
    last_consecutive: u16,
}

impl NackSequenceWindow {
    fn new(log2_size_minus_6: u8) -> Self {
        Self {
            packets: vec![0u64; 1 << log2_size_minus_6],
            size: 1 << (log2_size_minus_6 + 6),
            end: 0,
            started: false,
            last_consecutive: 0,
        }
    }

    // 直接沿用 webrtc 默认 generator 的环形接收窗口语义，避免我们再发明一套缺包判定。
    fn add(&mut self, seq: u16) {
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

    fn missing_seq_numbers(&self, skip_last_n: u16) -> Vec<u16> {
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

    fn missing_seq_numbers_in_range(&self, start: u16, end_exclusive: u16) -> Vec<u16> {
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

#[cfg(test)]
mod tests {
    use super::{
        detect_forward_gap, resolve_recovery_keyframe_action, NackSequenceWindow,
        RecoveryKeyframeAction,
    };

    #[test]
    fn nack_sequence_window_tracks_missing_and_wrap() {
        let mut window = NackSequenceWindow::new(1);
        window.add(10);
        window.add(11);
        window.add(13);
        assert_eq!(window.missing_seq_numbers(0), vec![12]);
        assert_eq!(window.missing_seq_numbers_in_range(10, 14), vec![12]);

        let mut wrapped = NackSequenceWindow::new(1);
        wrapped.add(u16::MAX);
        wrapped.add(0);
        wrapped.add(2);
        assert_eq!(wrapped.missing_seq_numbers(0), vec![1]);
    }

    #[test]
    fn recovery_keyframe_action_only_waits_after_repeated_sample_loss() {
        assert_eq!(
            resolve_recovery_keyframe_action(false, 1, 3, false),
            (false, RecoveryKeyframeAction::DropAndRequestKeyframe)
        );
        assert_eq!(
            resolve_recovery_keyframe_action(false, 2, 3, false),
            (true, RecoveryKeyframeAction::TriggerWaitKeyframe)
        );
        assert_eq!(
            resolve_recovery_keyframe_action(true, 0, 0, false),
            (true, RecoveryKeyframeAction::WaitKeyframe)
        );
        assert_eq!(
            resolve_recovery_keyframe_action(true, 0, 0, true),
            (false, RecoveryKeyframeAction::Submit)
        );
    }

    #[test]
    fn detect_forward_gap_ignores_old_out_of_order_packets() {
        assert_eq!(detect_forward_gap(None, 10), (Some(10), None));
        assert_eq!(detect_forward_gap(Some(10), 11), (Some(11), None));
        assert_eq!(detect_forward_gap(Some(10), 13), (Some(13), Some((11, 13))));
        assert_eq!(detect_forward_gap(Some(13), 12), (Some(13), None));
    }
}
