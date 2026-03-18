use bytes::Bytes;
use rtp::codecs::h264::H264Packet;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_remote::TrackRemote;
use webrtc_media::io::sample_builder::SampleBuilder;

use crate::media::video::types::{EncodedFrame, FrameValue, VideoCodec};
use crate::transport::webrtc::frame_deadline::FrameDeadlineTracker;
use crate::transport::webrtc::nack_scheduler::{NackScheduler, NackSchedulerConfig};
use crate::transport::webrtc::recovery::recovery_signal::VideoRecoverySignal;
use crate::XbxEngineMediaRuntimeStats;

mod nack;
mod source;

pub enum FrameSourceEvent {
    Frame(EncodedFrame),
    RecoverySignal(VideoRecoverySignal),
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
    pending_recovery_signal: Option<VideoRecoverySignal>,
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
            pending_recovery_signal: None,
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

    fn queue_recovery_signal(&mut self, signal: VideoRecoverySignal) {
        if self.pending_recovery_signal.is_none() {
            self.pending_recovery_signal = Some(signal);
        }
    }
}

#[derive(Clone, Copy)]
struct RecentRtpPacket {
    sequence: u16,
    rtp_timestamp: u32,
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

#[cfg(test)]
mod tests;
