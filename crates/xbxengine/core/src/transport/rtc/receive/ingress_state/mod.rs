use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState,
    XbxEngineMediaRuntimeStats,
};

use crate::media::video::types::{FrameRecoveryDisposition, FrameValue};
use crate::runtime_stats_sink::RuntimeStatsSink;
#[cfg(test)]
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_observed_at_ms, has_current_transport_await_issue_from_observation,
};
use crate::transport::rtc::stream::packet_types::RtcVideoRtpPacket;
use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::transport::rtc::recovery::contract::has_current_transport_await_issue_from_stats;
use crate::transport::rtc::stream::frame_cadence::TransportFrameDeadlineTracker;
pub(crate) use crate::transport::rtc::stream::nack_contract::NackSchedulerConfig;

use crate::transport::rtc::receive::nack_policy::{
    NACK_MAINTENANCE_TICK_INTERVAL_MS, RECOVERY_KEYFRAME_RETRY_INTERVAL_MS,
    RECOVERY_KEYFRAME_RETRY_TIMEOUT_MS,
};
use crate::transport::rtc::receive::ReceiverState;
use crate::transport::rtc::receive::ReceiverTraceLedger;

use crate::transport::rtc::capability::RtcTransportCapability;
use crate::transport::rtc::receive::{
    receiver_state_from_runtime, ReceiveCoreBody, ReceiveEngine, ReceiverObservation,
};
use crate::transport::rtc::recovery::contract::SparseIdrRhythm;
use crate::transport::rtc::stream::adapter_types::{
    TransportAdmissionObservation, TransportLossObservation, TransportObservation,
    VideoFramePipelineSources,
};

pub struct RtcVideoFrameSource {
    pub(crate) rx: tokio::sync::mpsc::Receiver<RtcVideoRtpPacket>,
    pub(crate) transport_observation_tx: tokio::sync::mpsc::UnboundedSender<TransportObservation>,
    pub(crate) runtime_stats: RuntimeStatsSink,
    pub(crate) _max_late_packets: u16,
    pub(crate) _jitter_buffer_max_delay: Duration,
    pub(crate) idle_timeout: std::time::Duration,
    pub(crate) idle_hint_cooldown: std::time::Duration,
    pub(crate) last_packet_time: std::time::Instant,
    pub(crate) last_idle_hint_time: Option<std::time::Instant>,
    pub(crate) pending_idle_timeout_since: Option<std::time::Instant>,
    pub(crate) pending_thin_stream_since: Option<std::time::Instant>,
    pub(crate) assembly_stall_timeout: std::time::Duration,
    pub(crate) thin_stream_packet_threshold: u16,
    pub(crate) nack_skip_last_n: u16,
    pub(crate) last_nack_skip_last_n_updated_at_ms: Option<f64>,
    pub(crate) recent_oos_depths: VecDeque<u16>,
    pub(crate) recent_oos_active_until_ms: Option<f64>,
    pub(crate) oos_event_count: u64,
    pub(crate) frame_oos_flags: VecDeque<(u32, bool)>,
    pub(crate) frame_head_missing_flags: VecDeque<(u32, bool)>,
    pub(crate) frame_drop_buckets: VecDeque<(u32, u16)>,
    pub(crate) frame_playout_base_times: VecDeque<(u32, std::time::Instant)>,
    pub(crate) frame_first_packet_sequences: VecDeque<(u32, u16)>,
    pub(crate) recent_head_missing_active_until_ms: Option<f64>,
    pub(crate) last_highest_rtp_sequence: Option<u16>,
    pub(crate) current_width: u32,
    pub(crate) current_height: u32,
    pub(crate) recent_rtp_packets: VecDeque<RecentRtpPacket>,
    pub(crate) current_media_ssrc: Option<u32>,
    pub(crate) local_rtcp_sender_ssrc: u32,
    pub(crate) packet_gap_observation_id: u64,
    pub(crate) transport_deadline_tracker: TransportFrameDeadlineTracker,
    pub(crate) nack_observation_id: u64,
    pub(crate) last_transport_observation: Option<TransportObservation>,
    pub(crate) last_transport_observation_at: Option<std::time::Instant>,
    pub(crate) trace_ledger: ReceiverTraceLedger,
    pub(crate) wait_keyframe_observation_cooldown: std::time::Duration,
    pub(crate) nack_maintenance_tick_interval: std::time::Duration,
    pub(crate) last_nack_maintenance_tick_at: std::time::Instant,
    pub(crate) waiting_recovery_keyframe_since_ms: Option<f64>,
    pub(crate) recovery_keyframe_retry_timeout_ms: f64,
    pub(crate) recovery_keyframe_retry_interval_ms: f64,
    pub(crate) next_recovery_keyframe_retry_at_ms: Option<f64>,
    pub(crate) recovery_keyframe_retry_count: u16,
    pub(crate) sample_loss_burst_count: u8,
    pub(crate) clean_samples_since_loss: u8,
    pub(crate) last_submitted_frame_value: FrameValue,
    pub(crate) nack_recovery_ewma_ms: f64,
    pub(crate) nack_late_ewma: f64,
    pub(crate) core_body_slot: Option<ReceiveCoreBody>,
    pub(crate) first_frame_acquisition_keyframe_request_count: u8,
    pub(crate) reinject_read_poll_count: u64,
    pub(crate) received_packet_count: u64,
    pub(crate) transport_observation_emit_count: u64,
    pub(crate) jitter_early_emit_enabled: bool,
    pub(crate) jitter_early_emit_wait: Duration,
    pub(crate) pending_marker_boundary: Option<PendingMarkerBoundary>,
    pub(crate) jitter_marker_seen_count: u64,
    pub(crate) jitter_early_emit_count: u64,
    pub(crate) jitter_head_missing_signal_count: u64,
    pub(crate) ingress_budget_materialized_count: u64,
    pub(crate) ingress_budget_fallback_count: u64,
    pub(crate) ingress_budget_unknown_rtt_count: u64,
    pub(crate) frame_boundary: Arc<Mutex<FrameBoundaryTracker>>,
    pub(crate) last_consumed_clean_anchor_epoch: Option<u64>,
    pub(crate) receiver_observation_id: u64,
}

pub struct RtcVideoTransportObservationSource {
    pub(crate) rx: tokio::sync::mpsc::UnboundedReceiver<TransportObservation>,
}

impl RtcVideoFrameSource {
    pub(crate) fn receive_core(&self) -> &ReceiveCoreBody {
        self.core_body_slot
            .as_ref()
            .expect("ReceiveCoreBody not initialized")
    }

    pub(crate) fn receive_core_mut(&mut self) -> &mut ReceiveCoreBody {
        self.core_body_slot
            .as_mut()
            .expect("ReceiveCoreBody not initialized")
    }

    pub fn new(
        rx: tokio::sync::mpsc::Receiver<RtcVideoRtpPacket>,
        transport_observation_tx: tokio::sync::mpsc::UnboundedSender<TransportObservation>,
        runtime_stats: Arc<std::sync::Mutex<XbxEngineMediaRuntimeStats>>,
        max_late_packets: u16,
        jitter_buffer_min_delay: Duration,
        jitter_buffer_max_delay: Duration,
        idle_timeout: std::time::Duration,
        nack_config: NackSchedulerConfig,
        transport_capability: Arc<dyn RtcTransportCapability>,
    ) -> Self {
        let frame_deadline_ms = nack_config.frame_deadline_ms;
        let jitter_buffer_max_delay = jitter_buffer_max_delay.max(jitter_buffer_min_delay);
        let receive_target = runtime_stats
            .lock()
            .ok()
            .and_then(|stats| stats.session_target_type.clone());
        let assembly_stall_timeout = idle_timeout
            .mul_f32(3.0)
            .clamp(Duration::from_millis(240), Duration::from_millis(600));
        let source = Self {
            rx,
            transport_observation_tx,
            runtime_stats: RuntimeStatsSink::new(runtime_stats),
            _max_late_packets: max_late_packets,
            _jitter_buffer_max_delay: jitter_buffer_max_delay,
            idle_timeout,
            idle_hint_cooldown: idle_timeout.max(std::time::Duration::from_millis(400)),
            last_packet_time: std::time::Instant::now(),
            last_idle_hint_time: None,
            pending_idle_timeout_since: None,
            pending_thin_stream_since: None,
            assembly_stall_timeout,
            thin_stream_packet_threshold: nack_config.burst_count.saturating_mul(6).max(18),
            nack_skip_last_n: 2,
            last_nack_skip_last_n_updated_at_ms: None,
            recent_oos_depths: VecDeque::with_capacity(64),
            recent_oos_active_until_ms: None,
            oos_event_count: 0,
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
            trace_ledger: ReceiverTraceLedger::new(),
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
            core_body_slot: Some(ReceiveCoreBody::new(
                ReceiveEngine::for_video_source(
                    receive_target,
                    max_late_packets,
                    jitter_buffer_max_delay,
                ),
                transport_capability,
            )),
            first_frame_acquisition_keyframe_request_count: 0,
            reinject_read_poll_count: 0,
            received_packet_count: 0,
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
            last_consumed_clean_anchor_epoch: None,
            receiver_observation_id: 0,
        };
        source
    }

    pub(crate) fn queue_transport_observation(&mut self, observation: TransportObservation) {
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
            self.sync_recovery_ledger_to_stats();
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
        if self.should_suppress_receiver_local_transport_observation(observation) {
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
        self.trace_ledger.record_frame_recovery(
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
        self.trace_ledger.observe_anchor_candidate(
            recovery_epoch,
            frame_rtp_timestamp,
            source_event,
            state,
            failure_reason,
            observed_at_ms,
        );
        if let Some(candidate) = self.trace_ledger.latest_anchor_candidate_ledger() {
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
        if let Some(entry) = self.trace_ledger.take_frame_recovery(frame_rtp_timestamp) {
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

    pub(crate) fn receiver_local_state(&self) -> ReceiverState {
        let has_gap = self.receive_core().receive_engine.has_active_gap();
        receiver_state_from_runtime(
            self.waiting_recovery_keyframe_since_ms.is_some(),
            has_gap,
            self.receive_core()
                .receive_engine
                .frame_assembler
                .assembled_count(),
        )
    }

    pub(super) fn is_blocking_non_keyframe_admission(&self) -> bool {
        self.waiting_recovery_keyframe_since_ms.is_some()
    }

    pub(super) fn note_ingress_waiting_rtp_marker(&self) {
        if !self.is_blocking_non_keyframe_admission() {
            return;
        }
        self.runtime_stats.update(|stats| {
            stats.ingress_waiting_rtp_marker_total =
                stats.ingress_waiting_rtp_marker_total.saturating_add(1);
        });
    }

    pub(super) fn note_ingress_waiting_idr_inspection(&self) {
        if !self.is_blocking_non_keyframe_admission() {
            return;
        }
        self.runtime_stats.update(|stats| {
            stats.ingress_waiting_idr_inspection_total =
                stats.ingress_waiting_idr_inspection_total.saturating_add(1);
        });
    }

    pub(super) fn note_ingress_idr_not_admitted(&self, insert_reason: &str) {
        self.runtime_stats.update(|stats| {
            stats.ingress_idr_not_admitted_total =
                stats.ingress_idr_not_admitted_total.saturating_add(1);
            stats.latest_ingress_idr_not_admitted_reason = Some(insert_reason.to_string());
        });
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
    }

    #[cfg(test)]
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

    #[cfg(test)]
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
        let _ = now_ms;
        if stats.transport_state != XbxEngineTransportStateDto::Connected {
            return false;
        }
        if !Self::first_frame_acquired(stats) {
            return false;
        }
        if has_current_transport_await_issue_from_stats(stats) {
            return false;
        }
        if allow_anchor_soft_request {
            let hard_keyframe_bootstrap = matches!(
                invalid_bootstrap_reason,
                Some("bootstrapMissingIdr" | "mixedIdrWithTrailingDelta")
            );
            if hard_keyframe_bootstrap {
                return false;
            }
            let soft_invalid_bootstrap = matches!(
                invalid_bootstrap_reason,
                Some("bootstrapMissingSps" | "bootstrapMissingPps")
            );
            if soft_invalid_bootstrap && invalid_bootstrap_metadata_ready {
                return true;
            }
        }
        if allow_non_anchor_soft_request {
            return true;
        }
        false
    }

    fn publish_receiver_observation(&mut self, now_ms: f64, bootstrap_reject: Option<String>) {
        let has_gap = self.receive_core().receive_engine.has_active_gap();
        let state = self.receiver_local_state();
        let gap_sequence = self
            .runtime_stats
            .read(|stats| {
                stats
                    .latest_video_timeline_observation
                    .as_ref()
                    .and_then(|obs| obs.gap.as_ref().and_then(|gap| gap.sequence))
            })
            .flatten();
        self.receiver_observation_id = self.receiver_observation_id.saturating_add(1);
        let observation = ReceiverObservation {
            nack_in_flight: has_gap,
            keyframe_request_pending: self.waiting_recovery_keyframe_since_ms.is_some(),
            bootstrap_reject_reason: bootstrap_reject,
        };
        self.runtime_stats.record_video_receiver_observation(
            crate::XbxEngineVideoReceiverObservation {
                observation_id: self.receiver_observation_id,
                receiver_state: state.as_str().to_string(),
                gap_sequence,
                gap_span: None,
                nack_in_flight: observation.nack_in_flight,
                keyframe_request_pending: observation.keyframe_request_pending,
                bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                observed_at_ms: now_ms,
            },
        );
    }

    fn keyframe_request_force_required(
        &self,
        soft: bool,
        _now_ms: f64,
        rhythm: SparseIdrRhythm,
    ) -> bool {
        if !soft {
            return true;
        }
        if rhythm.active && rhythm.pli_due {
            return true;
        }
        false
    }

    /// 薄封装：所有 keyframe 决策经 `plan_receive_feedback` / `execute_receive_feedback_keyframe`。
    pub(crate) fn request_receiver_local_keyframe(
        &mut self,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
        soft: bool,
    ) {
        self.record_video_timeline_observation(source_event, None, frame_rtp_timestamp, now_ms);
        let effective_rtt_ms = self
            .runtime_stats
            .read(|stats| stats.recovery_effective_rtt_ms.unwrap_or(200.0))
            .unwrap_or(200.0);
        let sparse_idr_rhythm = self.sparse_idr_rhythm_for_receive(now_ms);
        let force = self.keyframe_request_force_required(soft, now_ms, sparse_idr_rhythm);
        let decision = self.plan_receive_feedback(
            source_event,
            now_ms,
            effective_rtt_ms,
            crate::transport::rtc::receive::feedback_arbiter::NackPollSnapshot::default(),
            None,
            force,
            soft,
        );
        if decision.should_touch_keyframe_executor() {
            let _ = self.execute_receive_feedback_keyframe(
                decision,
                source_event,
                frame_rtp_timestamp,
                now_ms,
                force,
            );
        } else {
            self.record_receive_feedback_decision(decision, source_event, None);
        }
    }

    pub(super) fn record_video_timeline_observation(
        &mut self,
        source_event: &str,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) {
        self.nack_observation_id = self.nack_observation_id.saturating_add(1);
        let receiver_state = self.receiver_local_state();
        let observation = crate::transport::rtc::receive::timeline_projection::project_latest_video_timeline_observation(
            receiver_state,
            &self.trace_ledger,
            self.nack_observation_id,
            source_event,
            gap_sequence,
            frame_rtp_timestamp,
            now_ms,
        );
        self.runtime_stats
            .record_video_timeline_observation(observation);
        self.publish_receiver_observation(now_ms, None);
    }
}

#[cfg(test)]
pub(crate) fn test_transport_capability() -> Arc<dyn RtcTransportCapability> {
    Arc::new(crate::transport::rtc::capability::TestTransportCapability)
}

#[cfg(test)]
pub(crate) fn test_nack_scheduler_config() -> NackSchedulerConfig {
    NackSchedulerConfig {
        max_age_ms: 1_000,
        frame_deadline_ms: 120,
        burst_count: 2,
        retry_interval_ms: 20,
        max_retry_count: 3,
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
    use super::{
        should_suppress_transport_observation_for_runtime, video_ingress_channel_capacity,
        MAX_VIDEO_INGRESS_CHANNEL_CAPACITY, MIN_VIDEO_INGRESS_CHANNEL_CAPACITY,
    };
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

    #[test]
    fn video_ingress_channel_capacity_is_capped_for_low_latency() {
        assert_eq!(
            video_ingress_channel_capacity(1),
            MIN_VIDEO_INGRESS_CHANNEL_CAPACITY
        );
        assert_eq!(
            video_ingress_channel_capacity(96),
            MAX_VIDEO_INGRESS_CHANNEL_CAPACITY
        );
        assert_eq!(
            video_ingress_channel_capacity(8192),
            MAX_VIDEO_INGRESS_CHANNEL_CAPACITY
        );
    }
}

fn should_begin_transport_recovery_episode(observation: TransportObservation) -> bool {
    matches!(observation, TransportObservation::NackDeadlineExpired(_))
}

#[derive(Clone, Copy)]
pub(crate) struct RecentRtpPacket {
    pub(crate) sequence: u16,
    pub(crate) rtp_timestamp: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct PendingMarkerBoundary {
    pub(crate) sequence: u16,
    pub(crate) rtp_timestamp: u32,
    pub(crate) media_payload_type: u8,
    pub(crate) observed_at: std::time::Instant,
}

pub(crate) fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
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

pub(crate) struct FrameBoundaryTracker {
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

pub(crate) const UINT16SIZE_HALF: u16 = 1 << 15;
const MIN_VIDEO_INGRESS_CHANNEL_CAPACITY: usize = 64;
const MAX_VIDEO_INGRESS_CHANNEL_CAPACITY: usize = 64;

fn video_ingress_channel_capacity(requested: usize) -> usize {
    requested.clamp(
        MIN_VIDEO_INGRESS_CHANNEL_CAPACITY,
        MAX_VIDEO_INGRESS_CHANNEL_CAPACITY,
    )
}

pub(crate) fn build_rtc_video_frame_source(
    ingress_capacity: usize,
    runtime_stats: Arc<std::sync::Mutex<XbxEngineMediaRuntimeStats>>,
    max_late_packets: u16,
    jitter_buffer_min_delay: Duration,
    jitter_buffer_max_delay: Duration,
    idle_timeout: std::time::Duration,
    nack_config: NackSchedulerConfig,
    jitter_early_emit_enabled: bool,
    transport_capability: Arc<dyn RtcTransportCapability>,
) -> (
    Box<dyn crate::transport::rtc::stream::sink::RtcMediaSink>,
    VideoFramePipelineSources,
) {
    let channel_capacity = video_ingress_channel_capacity(ingress_capacity);
    let (tx, rx) = tokio::sync::mpsc::channel::<RtcVideoRtpPacket>(channel_capacity);
    let (transport_observation_tx, transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel::<TransportObservation>();
    let mut source = RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        runtime_stats.clone(),
        max_late_packets,
        jitter_buffer_min_delay,
        jitter_buffer_max_delay,
        idle_timeout,
        nack_config,
        transport_capability,
    );
    source.jitter_early_emit_enabled = jitter_early_emit_enabled;
    let frame_boundary = source.frame_boundary.clone();
    let sink = crate::transport::rtc::receive::rtx_sink::RtcVideoSourceSink::new(
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
            frame_source: Box::new(crate::transport::rtc::receive::RtcReceiveCore::new(source)),
            transport_observation_source: Box::new(observation_source),
        },
    )
}

mod decode;
mod feedback;

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

    /// replay harness 专用：绕过 decode gate 主循环，只泵送 rx 并清空 assembler。
    pub(crate) async fn drain_ingress_for_test(&mut self) {
        use std::time::Instant;

        loop {
            match tokio::time::timeout(Duration::from_millis(50), self.rx.recv()).await {
                Ok(None) => break,
                Ok(Some(packet)) => {
                    self.received_packet_count = self.received_packet_count.saturating_add(1);
                    let rtp = packet.to_rtp_packet();
                    let now = Instant::now();
                    self.last_packet_time = now;
                    self.receive_core_mut()
                        .receive_engine
                        .frame_assembler
                        .push_rtp(rtp, now);
                }
                Err(_) => break,
            }
        }
        while self
            .receive_core_mut()
            .receive_engine
            .frame_assembler
            .pop_access_unit()
            .is_some()
        {}
    }
}
