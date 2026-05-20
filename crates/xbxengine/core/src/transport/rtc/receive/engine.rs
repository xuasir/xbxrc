//! `RtcReceiveCore` 子模块聚合：packet buffer、RTP 组帧、NACK、keyframe、bootstrap。

use std::time::{Duration, Instant};

use xbxengine_protocol::XbxEngineTargetTypeDto;

use super::h264_bootstrap_tracker::H264BootstrapTracker;
use super::keyframe_requester::KeyframeRequester;
use super::nack_requester::NackRequester;
use super::packet_buffer::{PacketBuffer, SequenceObserveOutcome};
use super::rtp_frame_assembler::RtpFrameAssembler;
use super::timing::ReceiveTimingProfile;

/// receiver-local pre-decode 执行面（gap/NACK/组帧/bootstrap；裁决见 `ReceiverState` + decode gate）。
pub struct ReceiveEngine {
    pub packet_buffer: PacketBuffer,
    pub nack_requester: NackRequester,
    pub keyframe_requester: KeyframeRequester,
    pub bootstrap: H264BootstrapTracker,
    pub frame_assembler: RtpFrameAssembler,
    pub timing: ReceiveTimingProfile,
}

impl ReceiveEngine {
    pub fn new(
        timing: ReceiveTimingProfile,
        max_late_packets: u16,
        max_time_delay: Duration,
    ) -> Self {
        Self {
            packet_buffer: PacketBuffer::default(),
            nack_requester: NackRequester::new(timing),
            keyframe_requester: KeyframeRequester::new(timing),
            bootstrap: H264BootstrapTracker::default(),
            frame_assembler: RtpFrameAssembler::new(max_late_packets, max_time_delay),
            timing,
        }
    }

    pub fn for_video_source(
        target: Option<XbxEngineTargetTypeDto>,
        max_late_packets: u16,
        max_time_delay: Duration,
    ) -> Self {
        Self::new(
            ReceiveTimingProfile::for_target(target),
            max_late_packets,
            max_time_delay,
        )
    }

    pub fn observe_rtp_sequence(&mut self, sequence: u16, now_ms: f64) -> SequenceObserveOutcome {
        let mut outcome = self.packet_buffer.observe_sequence(sequence, now_ms);
        self.nack_requester
            .register_gaps(outcome.newly_opened_gaps.clone());
        self.packet_buffer.resolve_sequence(sequence);
        outcome.resolved_pending_nack = self.nack_requester.resolve(sequence);
        outcome
    }

    pub fn pending_nack_count(&self) -> usize {
        self.nack_requester.pending_count()
    }

    pub fn prune_pending_nack_in_range(&mut self, start: u16, end_exclusive: u16) -> Vec<u16> {
        self.nack_requester
            .prune_pending_in_range(start, end_exclusive)
    }

    pub fn has_active_gap(&self) -> bool {
        self.packet_buffer.has_active_gap()
    }

    pub fn poll_nack_maintenance(&mut self, now: Instant) -> (Vec<u16>, bool) {
        self.nack_requester.sync_from_buffer(&self.packet_buffer);
        self.nack_requester.poll_ready_sequences(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_gap_registers_nack_pending() {
        let mut engine = ReceiveEngine::for_video_source(None, 16, Duration::from_millis(80));
        engine.observe_rtp_sequence(100, 0.0);
        engine.observe_rtp_sequence(103, 1.0);
        assert!(engine.has_active_gap());
    }
}
