use bytes::Bytes;
use rtp::codecs::h264::H264Packet;
use std::sync::{Arc, Mutex};

use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_remote::TrackRemote;
use webrtc_media::io::sample_builder::SampleBuilder;

use crate::media::video::types::{EncodedFrame, FrameValue, VideoCodec};
use crate::transport::h264_resolution::parse_sps_dimensions_from_nal;
use crate::transport::webrtc::escalation::VideoEscalationReason;
use crate::transport::webrtc::frame_deadline::FrameDeadlineTracker;
use crate::transport::webrtc::nack_scheduler::{
    NackBatch, NackScheduler, NackSchedulerConfig, ResolvedNack,
};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoNackObservation};

pub enum FrameSourceEvent {
    Frame(EncodedFrame),
    PacketGapDetected {
        expected_sequence: u16,
        received_sequence: u16,
        missing_count: u16,
    },
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
    idle_timeout: std::time::Duration,
    idle_hint_cooldown: std::time::Duration,
    last_packet_time: std::time::Instant,
    assembling_frame_start: Option<std::time::Instant>,
    last_idle_hint_time: Option<std::time::Instant>,
    last_sequence: Option<u16>,
    nack_scheduler: NackScheduler,
    frame_deadline_tracker: FrameDeadlineTracker,
    nack_observation_id: u64,
    pending_escalation_hint: Option<(VideoEscalationReason, &'static str)>,
    severe_deadline_packet_threshold: usize,
    current_width: u32,
    current_height: u32,
}

impl WebrtcVideoAdapter {
    pub fn new(
        track: Arc<TrackRemote>,
        peer_connection: Arc<RTCPeerConnection>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        max_late_packets: u16,
        idle_timeout: std::time::Duration,
        nack_config: NackSchedulerConfig,
    ) -> Self {
        let frame_deadline_ms = nack_config.frame_deadline_ms;
        let burst_count = usize::from(nack_config.burst_count.max(1));
        Self {
            track,
            peer_connection,
            runtime_stats,
            sample_builder: SampleBuilder::new(max_late_packets, H264Packet::default(), 90_000),
            max_late_packets,
            idle_timeout,
            idle_hint_cooldown: idle_timeout.max(std::time::Duration::from_millis(400)),
            last_packet_time: std::time::Instant::now(),
            assembling_frame_start: None,
            last_idle_hint_time: None,
            last_sequence: None,
            nack_scheduler: NackScheduler::new(nack_config),
            frame_deadline_tracker: FrameDeadlineTracker::new(frame_deadline_ms),
            nack_observation_id: 0,
            pending_escalation_hint: None,
            // 大范围 deadline 失效通常不是“再试一次 keyframe”能解决的，
            // 这里提前标成 severe，交给统一 escalation ladder 处理。
            severe_deadline_packet_threshold: (burst_count * 32).max(128),
            current_width: 0,
            current_height: 0,
        }
    }

    async fn maybe_run_nack_maintenance(&mut self) {
        let now_ms = now_ms_f64();
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
                &expired_batch.sequences,
                0,
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

    async fn send_nack_batch(&mut self, action: &str, batch: &NackBatch, now_ms: f64) {
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
        self.record_nack_observation(action, &batch.sequences, batch.retry_count, now_ms);
    }

    fn record_nack_observation(
        &mut self,
        action: &str,
        sequences: &[u16],
        retry_count: u8,
        now_ms: f64,
    ) {
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
                first_sequence,
                last_sequence,
                packet_count: sequences.len().min(u16::MAX as usize) as u16,
                retry_count,
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
                first_sequence: resolved.sequence,
                last_sequence: resolved.sequence,
                packet_count: 1,
                retry_count: resolved.retry_count,
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

    fn queue_escalation_hint(&mut self, reason: VideoEscalationReason, label: &'static str) {
        if self.pending_escalation_hint.is_none() {
            self.pending_escalation_hint = Some((reason, label));
        }
    }
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
                    let payload = sample.data.to_vec();
                    // extract the actual sequence numbers from the sample if needed,
                    // but since sample builder popped it, it means it's complete.
                    let (is_keyframe, maybe_res) = parse_idr_and_sps(&payload);

                    let mut config_changed = false;
                    if let Some((w, h)) = maybe_res {
                        if w != self.current_width || h != self.current_height {
                            self.current_width = w;
                            self.current_height = h;
                            config_changed = true;
                        }
                    }

                    let playout_delay = std::time::Duration::from_millis(30);
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
                        value: if is_keyframe {
                            FrameValue::Keyframe
                        } else {
                            FrameValue::Delta
                        },
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

                if idle_timeout {
                    self.sample_builder =
                        SampleBuilder::new(self.max_late_packets, H264Packet::default(), 90_000);
                    self.assembling_frame_start = None;
                    self.last_packet_time = now;

                    if self
                        .last_idle_hint_time
                        .map_or(true, |t| now.duration_since(t) >= self.idle_hint_cooldown)
                    {
                        self.last_idle_hint_time = Some(now);
                        return Some(FrameSourceEvent::EscalationHint {
                            reason: VideoEscalationReason::AdapterIdleTimeout,
                            label: "adapterIdleTimeout",
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
                        }
                        let seq = rtp.header.sequence_number;
                        let now_ms = now_ms_f64();
                        if let Some(resolved) = self.nack_scheduler.resolve_sequence(seq, now_ms) {
                            self.record_nack_recovered(resolved, now_ms);
                        }
                        if let Some(previous_seq) = self.last_sequence {
                            let delta = classify_sequence_delta(previous_seq, seq);
                            if delta > 1 {
                                let missing_count = (delta - 1) as u16;
                                self.last_sequence = Some(seq);
                                self.sample_builder.push(rtp);
                                if let Some(initial_batch) = self.nack_scheduler.observe_gap(
                                    previous_seq.wrapping_add(1),
                                    seq,
                                    now_ms,
                                    Some(
                                        self.frame_deadline_tracker.next_deadline_for_value_at_ms(
                                            now_ms,
                                            FrameValue::Delta,
                                        ),
                                    ),
                                ) {
                                    if let Ok(mut stats) = self.runtime_stats.lock() {
                                        stats.inbound_video_packet_loss_estimate_total = stats
                                            .inbound_video_packet_loss_estimate_total
                                            .saturating_add(u64::from(missing_count));
                                    }
                                    self.send_nack_batch("sent", &initial_batch, now_ms).await;
                                }
                                return Some(FrameSourceEvent::PacketGapDetected {
                                    expected_sequence: previous_seq.wrapping_add(1),
                                    received_sequence: seq,
                                    missing_count,
                                });
                            }
                            if delta <= 0 {
                                // 乱序/重复包不更新 last_sequence，避免被误判成超大 gap。
                                self.sample_builder.push(rtp);
                                continue;
                            }
                        }
                        self.last_sequence = Some(seq);
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

fn classify_sequence_delta(previous_seq: u16, current_seq: u16) -> i32 {
    i32::from((current_seq.wrapping_sub(previous_seq)) as i16)
}

#[cfg(test)]
mod tests {
    use super::classify_sequence_delta;

    #[test]
    fn classify_sequence_delta_treats_small_reorder_as_negative() {
        assert_eq!(classify_sequence_delta(15348, 15346), -2);
        assert_eq!(classify_sequence_delta(15348, 15348), 0);
    }

    #[test]
    fn classify_sequence_delta_keeps_forward_progress_and_wrap() {
        assert_eq!(classify_sequence_delta(15348, 15350), 2);
        assert_eq!(classify_sequence_delta(u16::MAX, 0), 1);
    }
}
