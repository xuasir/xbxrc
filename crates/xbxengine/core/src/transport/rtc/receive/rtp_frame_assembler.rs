use std::time::{Duration, Instant};

use bytes::Bytes;
use rtc_media::io::sample_builder::SampleBuilder;
use rtc_rtp::codec::h264::H264Packet;
use rtc_rtp::packet::Packet;

/// RTP 组帧窗口进度（AU 由 `SampleBuilder` 产出）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FrameAssemblyState {
    #[default]
    Open,
    Complete,
    Incomplete,
}

/// receiver-local RTP → access unit 组帧（`SampleBuilder` + assembly 事实）。
pub struct RtpFrameAssembler {
    builder: SampleBuilder<H264Packet>,
    max_late_packets: u16,
    max_time_delay: Duration,
    pub state: FrameAssemblyState,
    started_at: Option<Instant>,
    packet_count: u16,
    assembled_count: u64,
}

#[derive(Clone, Debug)]
pub struct RtpAccessUnit {
    pub packet_timestamp: u32,
    pub payload: Vec<u8>,
    pub prev_dropped_packets: u16,
    pub prev_padding_packets: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct SyntheticMarkerBoundary {
    pub sequence: u16,
    pub rtp_timestamp: u32,
    pub media_payload_type: u8,
}

impl RtpFrameAssembler {
    pub fn new(max_late_packets: u16, max_time_delay: Duration) -> Self {
        Self {
            builder: build_sample_builder(max_late_packets, max_time_delay),
            max_late_packets,
            max_time_delay,
            state: FrameAssemblyState::Open,
            started_at: None,
            packet_count: 0,
            assembled_count: 0,
        }
    }

    pub fn push_rtp(&mut self, packet: Packet, now: Instant) {
        self.track_packet(now);
        self.builder.push(packet);
    }

    pub fn pop_access_unit(&mut self) -> Option<RtpAccessUnit> {
        let sample = self.builder.pop()?;
        self.on_frame_complete();
        Some(RtpAccessUnit {
            packet_timestamp: sample.packet_timestamp,
            payload: sample.data.to_vec(),
            prev_dropped_packets: sample.prev_dropped_packets,
            prev_padding_packets: sample.prev_padding_packets,
        })
    }

    pub fn reset_builder(&mut self) {
        self.builder = build_sample_builder(self.max_late_packets, self.max_time_delay);
        self.abandon_open_assembly();
    }

    /// jitter early emit：注入 AUD 合成边界包，触发上一帧 `pop()`。
    pub fn push_synthetic_aud_boundary(
        &mut self,
        boundary: SyntheticMarkerBoundary,
        media_ssrc: u32,
    ) {
        let synthetic = Packet {
            header: rtc_rtp::header::Header {
                version: 2,
                marker: false,
                payload_type: boundary.media_payload_type,
                sequence_number: boundary.sequence.wrapping_add(1),
                timestamp: boundary.rtp_timestamp.wrapping_add(1),
                ssrc: media_ssrc,
                ..Default::default()
            },
            payload: Bytes::from_static(&[0x09, 0xF0]),
        };
        self.builder.push(synthetic);
    }

    pub fn assembled_count(&self) -> u64 {
        self.assembled_count
    }

    pub fn packet_count(&self) -> u16 {
        self.packet_count
    }

    pub fn should_trigger_thin_stream_stall(
        &self,
        now: Instant,
        stall_timeout: Duration,
        thin_stream_packet_threshold: u16,
    ) -> bool {
        self.started_at.is_some_and(|started_at| {
            now.duration_since(started_at) >= stall_timeout
                && self.packet_count > 0
                && self.packet_count <= thin_stream_packet_threshold
        })
    }

    pub(crate) fn track_packet(&mut self, now: Instant) {
        if self.started_at.is_none() {
            self.started_at = Some(now);
            self.packet_count = 0;
            self.state = FrameAssemblyState::Open;
        }
        self.packet_count = self.packet_count.saturating_add(1);
    }

    pub(crate) fn on_frame_complete(&mut self) {
        self.assembled_count = self.assembled_count.saturating_add(1);
        self.started_at = None;
        self.packet_count = 0;
        self.state = FrameAssemblyState::Open;
    }

    pub fn abandon_open_assembly(&mut self) {
        self.started_at = None;
        self.packet_count = 0;
        self.state = FrameAssemblyState::Incomplete;
    }
}

pub(crate) fn build_sample_builder(
    max_late_packets: u16,
    max_time_delay: Duration,
) -> SampleBuilder<H264Packet> {
    SampleBuilder::new(max_late_packets, H264Packet::default(), 90_000)
        .with_max_time_delay(max_time_delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_stream_stall_requires_open_window_with_few_packets() {
        let mut assembler = RtpFrameAssembler::new(16, Duration::from_millis(80));
        let t0 = Instant::now();
        assembler.track_packet(t0);
        assembler.track_packet(t0);
        assert!(!assembler.should_trigger_thin_stream_stall(
            t0 + Duration::from_millis(50),
            Duration::from_millis(300),
            8,
        ));
        assert!(assembler.should_trigger_thin_stream_stall(
            t0 + Duration::from_millis(400),
            Duration::from_millis(300),
            8,
        ));
    }

    #[test]
    fn pop_sample_increments_assembled_count() {
        let mut assembler = RtpFrameAssembler::new(16, Duration::from_millis(80));
        assembler.on_frame_complete();
        assert_eq!(assembler.assembled_count(), 1);
        assert_eq!(assembler.packet_count(), 0);
    }
}
