use bytes::Bytes;
use rtp::codecs::h264::H264Packet;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::media::video::h264::inspection::H264AccessUnitInspector;
use webrtc::track::track_remote::TrackRemote;
use webrtc_media::io::sample_builder::SampleBuilder;

use crate::media::video::types::{AssembledVideoFrame, FrameValue, VideoCodec};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::webrtc::nack_scheduler::{NackScheduler, NackSchedulerConfig};
use crate::XbxEngineMediaRuntimeStats;

mod frame_cadence;
mod nack;
mod source;

use frame_cadence::TransportFrameDeadlineTracker;

pub trait TransportFeedbackPort: Send + Sync {
    fn send_transport_layer_nack<'a>(
        &'a self,
        media_ssrc: u32,
        sequences: &'a [u16],
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportAdmissionObservation {
    AwaitRecoveryKeyframe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportLossObservation {
    PacketLossDetected,
    RecoveryKeyframeRequested,
    AwaitRecoveryKeyframe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportObservation {
    Admission(TransportAdmissionObservation),
    Loss(TransportLossObservation),
    StreamIdleTimeout,
    StreamThinStall,
    NackDeadlineExpired { missing_packets: u16 },
    NackRecoveredLate,
}

pub enum FrameSourceEvent {
    // adapter 只负责把 RTP 样本整理成可消费的编码帧，不在这里承诺最终 playout budget。
    Frame(AssembledVideoFrame),
    TransportObservation(TransportObservation),
}

pub trait FrameSource: Send {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<FrameSourceEvent>> + Send + 'a>>;
}

pub struct WebrtcVideoAdapter {
    track: Arc<TrackRemote>,
    feedback_port: Arc<dyn TransportFeedbackPort>,
    runtime_stats: RuntimeStatsSink,
    sample_builder: SampleBuilder<H264Packet>,
    max_late_packets: u16,
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
    current_width: u32,
    current_height: u32,
    recent_rtp_packets: VecDeque<RecentRtpPacket>,
    packet_gap_observation_id: u64,
    transport_deadline_tracker: TransportFrameDeadlineTracker,
    nack_observation_id: u64,
    pending_transport_observation: Option<TransportObservation>,
    last_transport_observation: Option<TransportObservation>,
    last_transport_observation_at: Option<std::time::Instant>,
    waiting_for_recovery_keyframe: bool,
    wait_keyframe_observation_cooldown: std::time::Duration,
    sample_loss_burst_count: u8,
    clean_samples_since_loss: u8,
    last_submitted_frame_value: FrameValue,
    nack_recovery_ewma_ms: f64,
    nack_late_ewma: f64,
    h264_inspector: H264AccessUnitInspector,
    reinject_read_poll_count: u64,
}

impl WebrtcVideoAdapter {
    pub fn new(
        track: Arc<TrackRemote>,
        feedback_port: Arc<dyn TransportFeedbackPort>,
        runtime_stats: Arc<std::sync::Mutex<XbxEngineMediaRuntimeStats>>,
        max_late_packets: u16,
        jitter_buffer_min_delay: Duration,
        jitter_buffer_max_delay: Duration,
        idle_timeout: std::time::Duration,
        nack_config: NackSchedulerConfig,
    ) -> Self {
        let frame_deadline_ms = nack_config.frame_deadline_ms;
        let jitter_buffer_max_delay = jitter_buffer_max_delay.max(jitter_buffer_min_delay);
        let assembly_stall_timeout = idle_timeout
            .mul_f32(3.0)
            .clamp(Duration::from_millis(240), Duration::from_millis(600));
        Self {
            track,
            feedback_port,
            runtime_stats: RuntimeStatsSink::new(runtime_stats),
            sample_builder: build_sample_builder(max_late_packets, jitter_buffer_max_delay),
            max_late_packets,
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
            current_width: 0,
            current_height: 0,
            recent_rtp_packets: VecDeque::with_capacity(512),
            packet_gap_observation_id: 0,
            transport_deadline_tracker: TransportFrameDeadlineTracker::new(frame_deadline_ms),
            nack_observation_id: 0,
            pending_transport_observation: None,
            last_transport_observation: None,
            last_transport_observation_at: None,
            waiting_for_recovery_keyframe: false,
            wait_keyframe_observation_cooldown: Duration::from_millis(350),
            sample_loss_burst_count: 0,
            clean_samples_since_loss: 0,
            last_submitted_frame_value: FrameValue::new(false, false, 12 * 1024),
            nack_recovery_ewma_ms: 22.0,
            nack_late_ewma: 0.0,
            h264_inspector: H264AccessUnitInspector::new(),
            reinject_read_poll_count: 0,
        }
    }

    fn queue_transport_observation(&mut self, observation: TransportObservation) {
        let now = std::time::Instant::now();
        if self.should_suppress_transport_observation(observation, now) {
            return;
        }
        self.last_transport_observation = Some(observation);
        self.last_transport_observation_at = Some(now);

        if let Some(pending) = self.pending_transport_observation {
            if transport_observation_priority(observation)
                <= transport_observation_priority(pending)
            {
                return;
            }
        }
        if should_begin_transport_recovery_episode(observation) {
            self.runtime_stats.begin_transport_recovery_episode();
        }
        self.pending_transport_observation = Some(observation);
    }

    fn should_suppress_transport_observation(
        &self,
        observation: TransportObservation,
        now: std::time::Instant,
    ) -> bool {
        let is_wait_keyframe = matches!(
            observation,
            TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
                | TransportObservation::Loss(TransportLossObservation::AwaitRecoveryKeyframe)
        );
        if !is_wait_keyframe {
            return false;
        }
        let Some(last_observation) = self.last_transport_observation else {
            return false;
        };
        let was_wait_keyframe = matches!(
            last_observation,
            TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
                | TransportObservation::Loss(TransportLossObservation::AwaitRecoveryKeyframe)
        );
        if !was_wait_keyframe {
            return false;
        }
        self.last_transport_observation_at.is_some_and(|last_at| {
            now.duration_since(last_at) < self.wait_keyframe_observation_cooldown
        })
    }
}

fn transport_observation_priority(observation: TransportObservation) -> u8 {
    match observation {
        TransportObservation::NackDeadlineExpired { .. } => 6,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested) => 5,
        TransportObservation::Loss(TransportLossObservation::PacketLossDetected) => 4,
        TransportObservation::Loss(TransportLossObservation::AwaitRecoveryKeyframe)
        | TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe) => {
            3
        }
        TransportObservation::StreamThinStall | TransportObservation::StreamIdleTimeout => 2,
        TransportObservation::NackRecoveredLate => 1,
    }
}

fn should_begin_transport_recovery_episode(observation: TransportObservation) -> bool {
    matches!(
        observation,
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
            | TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
            | TransportObservation::Loss(TransportLossObservation::AwaitRecoveryKeyframe)
            | TransportObservation::StreamIdleTimeout
            | TransportObservation::StreamThinStall
            | TransportObservation::NackDeadlineExpired { .. }
    )
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
