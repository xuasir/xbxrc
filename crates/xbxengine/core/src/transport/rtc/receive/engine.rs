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
    pub _timing: ReceiveTimingProfile,
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
            _timing: timing,
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

    pub fn mark_sequence_recovered(
        &mut self,
        sequence: u16,
        source: super::nack_requester::RecoveredPacketSource,
    ) -> bool {
        self.packet_buffer.resolve_sequence(sequence);
        self.nack_requester.mark_recovered(sequence, source)
    }

    pub fn pending_nack_count(&self) -> usize {
        self.nack_requester.pending_count()
    }

    pub fn has_active_gap(&self) -> bool {
        self.packet_buffer.has_active_gap()
    }

    pub fn clear_recovery_state_after_decoded_anchor(&mut self) {
        self.packet_buffer.clear_gaps();
        self.nack_requester.clear_recovery_state();
    }

    pub fn poll_nack_maintenance(
        &mut self,
        now: Instant,
        effective_rtt_ms: f64,
        sparse_idr_rhythm: crate::transport::rtc::recovery::contract::SparseIdrRhythm,
    ) -> super::nack_requester::NackPollResult {
        let params = self._timing.nack_scheduling_params(effective_rtt_ms);
        self.nack_requester.sync_from_buffer(&self.packet_buffer);
        self.nack_requester.poll(&params, now, sparse_idr_rhythm)
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
