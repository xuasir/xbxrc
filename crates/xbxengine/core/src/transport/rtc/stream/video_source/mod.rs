use rtc_media::io::sample_builder::SampleBuilder;
use rtc_rtp::codec::h264::H264Packet;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::media::video::h264::inspection::H264AccessUnitInspector;
use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::transport::rtc::stream::nack_scheduler::{NackScheduler, NackSchedulerConfig};
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState,
    XbxEngineMediaRuntimeStats,
};

use super::packet_types::RtcVideoRtpPacket;
use super::sink::RtcRtcpSendPort;
use crate::media::video::types::{FrameRecoveryDisposition, FrameValue};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_observed_at_ms, has_current_transport_await_issue_from_observation,
    is_invalid_recovery_bootstrap_reject_reason,
};
use xbxengine_protocol::XbxEngineTransportStateDto;

pub(crate) mod nack;
pub(super) mod nack_policy;
pub(super) mod nack_window;
pub(crate) mod sink;
pub(crate) mod source;
#[cfg(test)]
pub(crate) mod test_fixtures;
pub(super) mod timeline;

use crate::transport::rtc::stream::frame_cadence::TransportFrameDeadlineTracker;

use self::nack_policy::{
    NACK_MAINTENANCE_TICK_INTERVAL_MS, RECOVERY_KEYFRAME_RETRY_INTERVAL_MS,
    RECOVERY_KEYFRAME_RETRY_TIMEOUT_MS,
};
use self::nack_window::NackSequenceWindow;
use self::timeline::VideoTimelineState;

use crate::transport::rtc::stream::adapter_types::{
    TransportAdmissionObservation, TransportLossObservation, TransportObservation,
    VideoFramePipelineSources,
};

pub struct RtcVideoFrameSource {
    rx: tokio::sync::mpsc::Receiver<RtcVideoRtpPacket>,
    transport_observation_tx: tokio::sync::mpsc::UnboundedSender<TransportObservation>,
    rtcp_port: Arc<dyn RtcRtcpSendPort>,
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
    pending_idle_timeout_since: Option<std::time::Instant>,
    pending_thin_stream_since: Option<std::time::Instant>,
    assembly_stall_timeout: std::time::Duration,
    thin_stream_packet_threshold: u16,
    nack_scheduler: NackScheduler,
    nack_window: NackSequenceWindow,
    nack_skip_last_n: u16,
    last_nack_skip_last_n_updated_at_ms: Option<f64>,
    recent_oos_depths: VecDeque<u16>,
    recent_oos_active_until_ms: Option<f64>,
    oos_event_count: u64,
    nack_window_overflow_count: u64,
    frame_oos_flags: VecDeque<(u32, bool)>,
    frame_head_missing_flags: VecDeque<(u32, bool)>,
    frame_drop_buckets: VecDeque<(u32, u16)>,
    frame_playout_base_times: VecDeque<(u32, std::time::Instant)>,
    frame_first_packet_sequences: VecDeque<(u32, u16)>,
    recent_head_missing_active_until_ms: Option<f64>,
    last_highest_rtp_sequence: Option<u16>,
    current_width: u32,
    current_height: u32,
    recent_rtp_packets: VecDeque<RecentRtpPacket>,
    current_media_ssrc: Option<u32>,
    local_rtcp_sender_ssrc: u32,
    packet_gap_observation_id: u64,
    transport_deadline_tracker: TransportFrameDeadlineTracker,
    nack_observation_id: u64,
    last_transport_observation: Option<TransportObservation>,
    last_transport_observation_at: Option<std::time::Instant>,
    timeline_state: VideoTimelineState,
    wait_keyframe_observation_cooldown: std::time::Duration,
    nack_maintenance_tick_interval: std::time::Duration,
    last_nack_maintenance_tick_at: std::time::Instant,
    waiting_recovery_keyframe_since_ms: Option<f64>,
    recovery_keyframe_retry_timeout_ms: f64,
    recovery_keyframe_retry_interval_ms: f64,
    next_recovery_keyframe_retry_at_ms: Option<f64>,
    recovery_keyframe_retry_count: u16,
    sample_loss_burst_count: u8,
    clean_samples_since_loss: u8,
    last_submitted_frame_value: FrameValue,
    nack_recovery_ewma_ms: f64,
    nack_late_ewma: f64,
    h264_inspector: H264AccessUnitInspector,
    first_frame_acquisition_keyframe_request_count: u8,
    reinject_read_poll_count: u64,
    received_packet_count: u64,
    assembled_frame_count: u64,
    transport_observation_emit_count: u64,
    jitter_early_emit_enabled: bool,
    jitter_early_emit_wait: Duration,
    pending_marker_boundary: Option<PendingMarkerBoundary>,
    jitter_marker_seen_count: u64,
    jitter_early_emit_count: u64,
    jitter_head_missing_signal_count: u64,
    ingress_budget_materialized_count: u64,
    ingress_budget_fallback_count: u64,
    ingress_budget_unknown_rtt_count: u64,
    frame_boundary: Arc<Mutex<FrameBoundaryTracker>>,
    last_consumed_clean_anchor_epoch: u64,
}

const SOURCE_SERVICEABLE_DECODE_MAX_AGE_MS: f64 = 180.0;
const SOURCE_SERVICEABLE_PRESENT_MAX_AGE_MS: f64 = 220.0;

pub struct RtcVideoTransportObservationSource {
    pub(crate) rx: tokio::sync::mpsc::UnboundedReceiver<TransportObservation>,
}

impl RtcVideoFrameSource {
    pub fn new(
        rx: tokio::sync::mpsc::Receiver<RtcVideoRtpPacket>,
        transport_observation_tx: tokio::sync::mpsc::UnboundedSender<TransportObservation>,
        rtcp_port: Arc<dyn RtcRtcpSendPort>,
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
            rx,
            transport_observation_tx,
            rtcp_port,
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
            pending_idle_timeout_since: None,
            pending_thin_stream_since: None,
            assembly_stall_timeout,
            thin_stream_packet_threshold: nack_config.burst_count.saturating_mul(6).max(18),
            nack_scheduler: NackScheduler::new(nack_config),
            nack_window: NackSequenceWindow::new(13 - 6),
            nack_skip_last_n: 2,
            last_nack_skip_last_n_updated_at_ms: None,
            recent_oos_depths: VecDeque::with_capacity(64),
            recent_oos_active_until_ms: None,
            oos_event_count: 0,
            nack_window_overflow_count: 0,
            frame_oos_flags: VecDeque::with_capacity(64),
            frame_head_missing_flags: VecDeque::with_capacity(64),
            frame_drop_buckets: VecDeque::with_capacity(64),
            frame_playout_base_times: VecDeque::with_capacity(64),
            frame_first_packet_sequences: VecDeque::with_capacity(64),
            recent_head_missing_active_until_ms: None,
            last_highest_rtp_sequence: None,
            current_width: 0,
            current_height: 0,
            recent_rtp_packets: VecDeque::with_capacity(512),
            current_media_ssrc: None,
            local_rtcp_sender_ssrc: generate_local_rtcp_sender_ssrc(),
            packet_gap_observation_id: 0,
            transport_deadline_tracker: TransportFrameDeadlineTracker::new(frame_deadline_ms),
            nack_observation_id: 0,
            last_transport_observation: None,
            last_transport_observation_at: None,
            timeline_state: VideoTimelineState::new(),
            wait_keyframe_observation_cooldown: Duration::from_millis(350),
            nack_maintenance_tick_interval: Duration::from_millis(
                NACK_MAINTENANCE_TICK_INTERVAL_MS,
            ),
            last_nack_maintenance_tick_at: std::time::Instant::now(),
            waiting_recovery_keyframe_since_ms: None,
            recovery_keyframe_retry_timeout_ms: RECOVERY_KEYFRAME_RETRY_TIMEOUT_MS,
            recovery_keyframe_retry_interval_ms: RECOVERY_KEYFRAME_RETRY_INTERVAL_MS,
            next_recovery_keyframe_retry_at_ms: None,
            recovery_keyframe_retry_count: 0,
            sample_loss_burst_count: 0,
            clean_samples_since_loss: 0,
            last_submitted_frame_value: FrameValue::new(false, false, 12 * 1024),
            nack_recovery_ewma_ms: 22.0,
            nack_late_ewma: 0.0,
            h264_inspector: H264AccessUnitInspector::new(),
            first_frame_acquisition_keyframe_request_count: 0,
            reinject_read_poll_count: 0,
            received_packet_count: 0,
            assembled_frame_count: 0,
            transport_observation_emit_count: 0,
            jitter_early_emit_enabled: false,
            jitter_early_emit_wait: Duration::from_millis(3),
            pending_marker_boundary: None,
            jitter_marker_seen_count: 0,
            jitter_early_emit_count: 0,
            jitter_head_missing_signal_count: 0,
            ingress_budget_materialized_count: 0,
            ingress_budget_fallback_count: 0,
            ingress_budget_unknown_rtt_count: 0,
            frame_boundary: Arc::new(Mutex::new(FrameBoundaryTracker::new())),
            last_consumed_clean_anchor_epoch: 0,
        }
    }

    fn queue_transport_observation(&mut self, observation: TransportObservation) {
        let now = std::time::Instant::now();
        if self.should_suppress_transport_observation(observation, now) {
            return;
        }
        self.last_transport_observation = Some(observation);
        self.last_transport_observation_at = Some(now);
        // adapterIdleTimeout / thin stream 只上行 MediaFact，由 policy 决定是否进入 recovery episode；避免源侧抢跑抬 epoch。
        if should_begin_transport_recovery_episode(observation) {
            self.runtime_stats
                .begin_transport_recovery_episode(now_ms_f64());
        }
        let _ = self.transport_observation_tx.send(observation);
        self.transport_observation_emit_count =
            self.transport_observation_emit_count.saturating_add(1);
        if self.transport_observation_emit_count == 1
            || self.transport_observation_emit_count.is_power_of_two()
        {
            crate::xbx_log_info!(
                "[RtcVideoFrameSource] queued transport observation count={} observation={:?}",
                self.transport_observation_emit_count,
                observation
            );
        }
    }

    fn should_suppress_transport_observation(
        &self,
        observation: TransportObservation,
        now: std::time::Instant,
    ) -> bool {
        let transport_state = self
            .runtime_stats
            .read(|stats| stats.transport_state.clone())
            .unwrap_or(XbxEngineTransportStateDto::New);
        if should_suppress_transport_observation_for_runtime(transport_state, observation) {
            return true;
        }
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

    pub(super) fn record_frame_recovery_ledger(
        &mut self,
        frame_rtp_timestamp: Option<u32>,
        frame_playout_deadline_at_ms: Option<f64>,
        frame_recovery_disposition: FrameRecoveryDisposition,
        frame_unrecoverable_reason: Option<&str>,
        budget_context: FrameBudgetContext,
        observed_at_ms: f64,
    ) {
        let Some(frame_rtp_timestamp) = frame_rtp_timestamp else {
            return;
        };
        self.nack_observation_id = self.nack_observation_id.saturating_add(1);
        self.runtime_stats.record_frame_recovery_observation(
            crate::XbxEngineFrameRecoveryObservation {
                observation_id: self.nack_observation_id,
                action: "ledgerWrite".to_string(),
                frame_rtp_timestamp,
                frame_playout_deadline_at_ms,
                frame_recovery_disposition: frame_recovery_disposition
                    .render_label()
                    .map(str::to_string),
                frame_unrecoverable_reason: frame_unrecoverable_reason.map(str::to_string),
                frame_budget: None,
                observed_at_ms,
            },
        );
        self.timeline_state.record_frame_recovery(
            frame_rtp_timestamp,
            frame_playout_deadline_at_ms,
            frame_recovery_disposition,
            frame_unrecoverable_reason,
            budget_context,
        );
        self.record_video_timeline_observation(
            "frame-recovery-ledger-write",
            None,
            Some(frame_rtp_timestamp),
            observed_at_ms,
        );
    }

    pub(super) fn record_anchor_candidate_ledger(
        &mut self,
        frame_rtp_timestamp: Option<u32>,
        source_event: &str,
        state: XbxEngineAnchorCandidateState,
        failure_reason: Option<XbxEngineAnchorCandidateFailureReason>,
        observed_at_ms: f64,
    ) {
        let recovery_epoch = self
            .runtime_stats
            .read(|stats| stats.transport_recovery_epoch)
            .unwrap_or(0);
        self.timeline_state.observe_anchor_candidate(
            recovery_epoch,
            frame_rtp_timestamp,
            source_event,
            state,
            failure_reason,
            observed_at_ms,
        );
        if let Some(candidate) = self.timeline_state.latest_anchor_candidate_ledger() {
            self.runtime_stats.record_anchor_candidate_ledger(
                candidate.recovery_epoch,
                candidate.frame_rtp_timestamp,
                candidate.state,
                candidate.source_event.as_str(),
                candidate.failure_reason,
                candidate.observed_at_ms,
            );
        }
    }

    pub(super) fn take_frame_recovery_ledger(
        &mut self,
        frame_rtp_timestamp: u32,
    ) -> (
        Option<f64>,
        FrameRecoveryDisposition,
        Option<String>,
        Option<FrameBudgetContext>,
    ) {
        if let Some(entry) = self.timeline_state.take_frame_recovery(frame_rtp_timestamp) {
            self.nack_observation_id = self.nack_observation_id.saturating_add(1);
            self.runtime_stats.record_frame_recovery_observation(
                crate::XbxEngineFrameRecoveryObservation {
                    observation_id: self.nack_observation_id,
                    action: "ledgerConsume".to_string(),
                    frame_rtp_timestamp,
                    frame_playout_deadline_at_ms: entry.frame_playout_deadline_at_ms,
                    frame_recovery_disposition: entry
                        .frame_recovery_disposition
                        .render_label()
                        .map(str::to_string),
                    frame_unrecoverable_reason: entry.frame_unrecoverable_reason.clone(),
                    frame_budget: None,
                    observed_at_ms: now_ms_f64(),
                },
            );
            self.record_video_timeline_observation(
                "frame-recovery-ledger-consume",
                None,
                Some(frame_rtp_timestamp),
                now_ms_f64(),
            );
            return (
                entry.frame_playout_deadline_at_ms,
                entry.frame_recovery_disposition,
                entry.frame_unrecoverable_reason,
                Some(entry.budget_context),
            );
        }
        (None, FrameRecoveryDisposition::Steady, None, None)
    }

    pub(super) fn is_blocking_non_keyframe_admission(&self) -> bool {
        self.timeline_state.chain_requires_recovery_anchor()
    }

    pub(super) fn set_is_blocking_non_keyframe_admission(&mut self, waiting: bool) {
        let now_ms = now_ms_f64();
        if waiting {
            if self.waiting_recovery_keyframe_since_ms.is_none() {
                self.waiting_recovery_keyframe_since_ms = Some(now_ms);
            }
            if self.next_recovery_keyframe_retry_at_ms.is_none() {
                self.next_recovery_keyframe_retry_at_ms =
                    Some(now_ms + self.recovery_keyframe_retry_timeout_ms);
            }
        } else {
            self.waiting_recovery_keyframe_since_ms = None;
            self.next_recovery_keyframe_retry_at_ms = None;
            self.recovery_keyframe_retry_count = 0;
        }
        self.timeline_state.apply_wait_keyframe_gate(waiting);
    }

    fn has_serviceable_output_for_soft_recovery_request(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> bool {
        if stats.transport_state != XbxEngineTransportStateDto::Connected
            || stats.video_renderer_stalled == Some(true)
        {
            return false;
        }
        let track_attached = stats
            .latest_video_track_status
            .as_ref()
            .is_some_and(|status| {
                status.state == "remoteTrackAttached" && status.video_bytes_total > 0
            });
        if !track_attached {
            return false;
        }
        let decode_fresh = stats
            .latest_video_decode_ok_time_ms
            .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= SOURCE_SERVICEABLE_DECODE_MAX_AGE_MS);
        let present_fresh = stats
            .latest_video_host_present_time_ms
            .is_some_and(|at_ms| {
                (now_ms - at_ms).max(0.0) <= SOURCE_SERVICEABLE_PRESENT_MAX_AGE_MS
            });
        decode_fresh && present_fresh
    }

    fn has_current_transport_await_issue_in_source(stats: &XbxEngineMediaRuntimeStats) -> bool {
        let Some(timeline) = stats.latest_video_timeline_observation.as_ref() else {
            return false;
        };
        has_current_transport_await_issue_from_observation(
            timeline,
            current_clean_anchor_observed_at_ms(
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                stats.video_anchor_clean_source_event.as_deref(),
                stats.transport_recovery_epoch,
            ),
        )
    }

    pub(super) fn should_rearm_clean_anchor_for_transport_await(
        stats: &XbxEngineMediaRuntimeStats,
    ) -> bool {
        current_clean_anchor_observed_at_ms(
            stats.video_anchor_clean_epoch,
            stats.video_anchor_clean_observed_at_ms,
            stats.video_anchor_clean_source_event.as_deref(),
            stats.transport_recovery_epoch,
        )
        .is_none()
            && Self::has_current_transport_await_issue_in_source(stats)
    }

    pub(super) fn should_soft_request_recovery_keyframe(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        invalid_bootstrap_reason: Option<&str>,
        invalid_bootstrap_metadata_ready: bool,
        allow_non_anchor_soft_request: bool,
        allow_anchor_soft_request: bool,
    ) -> bool {
        let has_current_clean_anchor = current_clean_anchor_observed_at_ms(
            stats.video_anchor_clean_epoch,
            stats.video_anchor_clean_observed_at_ms,
            stats.video_anchor_clean_source_event.as_deref(),
            stats.transport_recovery_epoch,
        )
        .is_some();
        if !has_current_clean_anchor
            || Self::has_current_transport_await_issue_in_source(stats)
            || !Self::has_serviceable_output_for_soft_recovery_request(stats, now_ms)
        {
            return false;
        }
        let invalid_bootstrap_ready =
            is_invalid_recovery_bootstrap_reject_reason(invalid_bootstrap_reason)
                && invalid_bootstrap_metadata_ready;
        invalid_bootstrap_ready || allow_non_anchor_soft_request || allow_anchor_soft_request
    }

    pub(super) fn record_video_timeline_observation(
        &mut self,
        source_event: &str,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) {
        self.nack_observation_id = self.nack_observation_id.saturating_add(1);
        let observation = self.timeline_state.snapshot_for_observation(
            self.nack_observation_id,
            source_event,
            gap_sequence,
            frame_rtp_timestamp,
            now_ms,
        );
        self.runtime_stats
            .record_video_timeline_observation(observation);
    }
}

fn should_suppress_transport_observation_for_runtime(
    transport_state: XbxEngineTransportStateDto,
    observation: TransportObservation,
) -> bool {
    let idle_or_thin_stall = matches!(
        observation,
        TransportObservation::StreamIdleTimeout | TransportObservation::StreamThinStall
    );
    if !idle_or_thin_stall {
        return false;
    }
    // 连接已关闭后，idle/thin-stall 继续上报只会挤占连接域信号。
    transport_state == XbxEngineTransportStateDto::Closed
}

#[cfg(test)]
mod tests {
    use super::should_suppress_transport_observation_for_runtime;
    use crate::transport::rtc::stream::adapter_types::TransportObservation;
    use xbxengine_protocol::XbxEngineTransportStateDto;

    #[test]
    fn closed_transport_suppresses_idle_observation_noise() {
        assert!(should_suppress_transport_observation_for_runtime(
            XbxEngineTransportStateDto::Closed,
            TransportObservation::StreamIdleTimeout,
        ));
    }

    #[test]
    fn non_closed_transport_keeps_thin_stall_signal() {
        assert!(!should_suppress_transport_observation_for_runtime(
            XbxEngineTransportStateDto::Connecting,
            TransportObservation::StreamThinStall,
        ));
    }
}

fn should_begin_transport_recovery_episode(observation: TransportObservation) -> bool {
    matches!(
        observation,
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
            | TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
            | TransportObservation::Loss(TransportLossObservation::AwaitRecoveryKeyframe)
            | TransportObservation::NackDeadlineExpired(_)
    )
}

#[derive(Clone, Copy)]
struct RecentRtpPacket {
    sequence: u16,
    rtp_timestamp: u32,
}

#[derive(Clone, Copy)]
struct PendingMarkerBoundary {
    sequence: u16,
    rtp_timestamp: u32,
    media_payload_type: u8,
    observed_at: std::time::Instant,
}

pub(super) fn build_sample_builder(
    max_late_packets: u16,
    max_time_delay: Duration,
) -> SampleBuilder<H264Packet> {
    SampleBuilder::new(max_late_packets, H264Packet::default(), 90_000)
        .with_max_time_delay(max_time_delay)
}

pub(super) fn now_ms_f64() -> f64 {
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

const UINT32SIZE_HALF: u32 = 0x8000_0000;
const FRAME_BOUNDARY_COMPLETED_CAPACITY: usize = 8;
const FRAME_BOUNDARY_STALE_FRAME_COUNT: u32 = 3;
const FRAME_BOUNDARY_STALE_TS_GAP: u32 = 3 * 3000;
const FRAME_BOUNDARY_TIMEOUT_MS: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FramePriority {
    Normal,
    High,
}

#[derive(Clone, Debug)]
struct CompletedFrame {
    rtp_timestamp: u32,
    seq_range: (u16, u16),
    #[allow(dead_code)]
    completed_at: Instant,
    #[allow(dead_code)]
    priority: FramePriority,
}

#[derive(Clone, Debug)]
struct ActiveFrame {
    #[allow(dead_code)]
    rtp_timestamp: u32,
    first_seq: u16,
    last_seq: u16,
    priority: FramePriority,
    first_seen_at: Instant,
    marker_seen: bool,
}

pub(super) struct FrameBoundaryTracker {
    completed_frames: VecDeque<CompletedFrame>,
    active_frames: HashMap<u32, ActiveFrame>,
    highest_timestamp_seen: Option<u32>,
}

impl FrameBoundaryTracker {
    pub(super) fn new() -> Self {
        Self {
            completed_frames: VecDeque::with_capacity(FRAME_BOUNDARY_COMPLETED_CAPACITY),
            active_frames: HashMap::new(),
            highest_timestamp_seen: None,
        }
    }

    pub(super) fn on_packet_arrived(&mut self, seq: u16, ts: u32, marker: bool, is_priority: bool) {
        if self.highest_timestamp_seen.is_none()
            || ts.wrapping_sub(self.highest_timestamp_seen.unwrap()) < UINT32SIZE_HALF
        {
            self.highest_timestamp_seen = Some(ts);
        }

        let active = self.active_frames.entry(ts).or_insert_with(|| ActiveFrame {
            rtp_timestamp: ts,
            first_seq: seq,
            last_seq: seq,
            priority: if is_priority {
                FramePriority::High
            } else {
                FramePriority::Normal
            },
            first_seen_at: Instant::now(),
            marker_seen: false,
        });

        if seq.wrapping_sub(active.first_seq) >= UINT16SIZE_HALF {
            active.first_seq = seq;
        }
        if seq.wrapping_sub(active.last_seq) < UINT16SIZE_HALF {
            active.last_seq = seq;
        }

        if is_priority {
            active.priority = FramePriority::High;
        }

        if marker {
            active.marker_seen = true;
        }
    }

    pub(super) fn maybe_finalize_frames(&mut self, now: Instant) {
        let highest_ts = self.highest_timestamp_seen.unwrap_or(0);

        self.active_frames.retain(|&ts, active| {
            let marker_confirmed = active.marker_seen
                && ts != highest_ts
                && highest_ts.wrapping_sub(ts) < UINT32SIZE_HALF;

            let ts_gap = highest_ts.wrapping_sub(ts);
            let stale_by_timestamp =
                ts_gap > FRAME_BOUNDARY_STALE_TS_GAP && ts_gap < UINT32SIZE_HALF;

            let stale_by_time = now.duration_since(active.first_seen_at)
                > Duration::from_millis(FRAME_BOUNDARY_TIMEOUT_MS);

            if marker_confirmed || stale_by_timestamp || stale_by_time {
                if self.completed_frames.len() >= FRAME_BOUNDARY_COMPLETED_CAPACITY {
                    self.completed_frames.pop_front();
                }
                self.completed_frames.push_back(CompletedFrame {
                    rtp_timestamp: ts,
                    seq_range: (active.first_seq, active.last_seq),
                    completed_at: now,
                    priority: active.priority,
                });
                false
            } else {
                true
            }
        });
    }

    pub(super) fn is_packet_stale(&self, ts: u32, seq: u16, is_primary: bool) -> bool {
        if is_primary {
            return false;
        }

        let Some(highest_ts) = self.highest_timestamp_seen else {
            return false;
        };

        let ts_gap = highest_ts.wrapping_sub(ts);

        if ts_gap == 0 || ts_gap >= UINT32SIZE_HALF {
            return false;
        }

        if self.active_frames.contains_key(&ts) {
            return false;
        }

        for completed in self
            .completed_frames
            .iter()
            .rev()
            .take(FRAME_BOUNDARY_STALE_FRAME_COUNT as usize)
        {
            if completed.rtp_timestamp == ts {
                let in_range = seq.wrapping_sub(completed.seq_range.0)
                    <= completed.seq_range.1.wrapping_sub(completed.seq_range.0);
                return in_range;
            }
        }

        ts_gap > FRAME_BOUNDARY_STALE_TS_GAP
    }

    pub(super) fn get_frame_priority(&self, ts: u32) -> Option<FramePriority> {
        self.active_frames.get(&ts).map(|active| active.priority)
    }
}

pub(super) const UINT16SIZE_HALF: u16 = 1 << 15;

pub(crate) fn build_rtc_video_frame_source(
    ingress_capacity: usize,
    rtcp_port: Arc<dyn super::sink::RtcRtcpSendPort>,
    runtime_stats: Arc<std::sync::Mutex<XbxEngineMediaRuntimeStats>>,
    max_late_packets: u16,
    jitter_buffer_min_delay: Duration,
    jitter_buffer_max_delay: Duration,
    idle_timeout: std::time::Duration,
    nack_config: NackSchedulerConfig,
    jitter_early_emit_enabled: bool,
) -> (
    Box<dyn super::sink::RtcMediaSink>,
    VideoFramePipelineSources,
) {
    let (tx, rx) = tokio::sync::mpsc::channel::<RtcVideoRtpPacket>(ingress_capacity.max(256));
    let (transport_observation_tx, transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel::<TransportObservation>();
    let mut source = RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        rtcp_port,
        runtime_stats.clone(),
        max_late_packets,
        jitter_buffer_min_delay,
        jitter_buffer_max_delay,
        idle_timeout,
        nack_config,
    );
    source.jitter_early_emit_enabled = jitter_early_emit_enabled;
    let frame_boundary = source.frame_boundary.clone();
    let sink = sink::RtcVideoSourceSink::new(
        tx,
        RuntimeStatsSink::new(runtime_stats.clone()),
        frame_boundary,
    );
    let observation_source = RtcVideoTransportObservationSource {
        rx: transport_observation_rx,
    };
    (
        Box::new(sink),
        VideoFramePipelineSources {
            frame_source: Box::new(source),
            transport_observation_source: Box::new(observation_source),
        },
    )
}

fn generate_local_rtcp_sender_ssrc() -> u32 {
    let seed = now_ms_f64() as u32;
    if seed == 0 {
        1
    } else {
        seed
    }
}

#[cfg(test)]
impl RtcVideoFrameSource {
    pub(crate) fn set_jitter_early_emit_enabled(&mut self, enabled: bool) {
        self.jitter_early_emit_enabled = enabled;
    }
}
