use crate::transport::rtc::receive::nack_maintenance::gap_transport_evidence;
use crate::transport::rtc::receive::now_ms_f64;
use crate::transport::rtc::receive::{DecodeGate, DecodeGateDecision, SequenceObserveOutcome};

use super::decode_gate_eval::detect_forward_gap;
use crate::media::video::types::AssembledVideoFrame;
use crate::transport::rtc::receive::{RtcVideoFrameSource, RtcVideoTransportObservationSource};
use crate::transport::rtc::stream::adapter_types::{
    FrameSource, TransportObservation, TransportObservationSource,
};
use crate::transport::rtc::stream::packet_types::RtcVideoIngressKind;

use crate::XbxEngineAnchorCandidateState;
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

const OOS_ACTIVITY_COOLDOWN_MS: f64 = 30_000.0;
const OOS_DEPTH_WINDOW_CAPACITY: usize = 64;
const OOS_SKIP_LAST_N_REFRESH_INTERVAL_MS: f64 = 200.0;
const FRAME_OOS_TRACK_CAPACITY: usize = 64;

impl RtcVideoFrameSource {
    pub(crate) fn resolve_effective_idle_controls(
        &self,
    ) -> (std::time::Duration, std::time::Duration) {
        let (session_target_type, feedback_interval_ms) = self
            .runtime_stats
            .read(|stats| {
                (
                    stats.session_target_type.clone(),
                    stats
                        .latest_video_twcc_observation
                        .as_ref()
                        .and_then(|observation| observation.feedback_interval_ms),
                )
            })
            .unwrap_or((None, None));
        resolve_effective_idle_controls(
            self.idle_timeout,
            self.idle_hint_cooldown,
            session_target_type.as_ref(),
            feedback_interval_ms,
        )
    }

    pub(crate) fn should_absorb_idle_timeout(&self, idle_timeout: std::time::Duration) -> bool {
        if self.is_blocking_non_keyframe_admission() {
            return false;
        }
        let now_ms = now_ms_f64();
        self.runtime_stats
            .read(|stats| {
                should_absorb_idle_timeout_for_steady_gap(
                    stats.transport_state.clone(),
                    stats.transport_recovery_epoch,
                    stats.video_anchor_clean_epoch,
                    stats.video_anchor_clean_source_event.as_deref(),
                    stats.latest_video_host_present_time_ms,
                    stats.latest_video_decode_ok_time_ms,
                    stats.video_renderer_stalled,
                    stats.video_decoder_stalled,
                    now_ms,
                    idle_timeout,
                )
            })
            .unwrap_or(false)
    }

    pub(crate) fn on_rtp_sequence_observed(
        &mut self,
        outcome: &SequenceObserveOutcome,
        rtp_timestamp: u32,
        now_ms: f64,
    ) {
        if !outcome.newly_opened_gaps.is_empty() {
            self.trace_ledger.observe_gap(
                &outcome.newly_opened_gaps,
                now_ms,
                Some(rtp_timestamp),
                "unknown",
                "unknown",
            );
            if outcome.is_reorder {
                self.trace_ledger.mark_gap_reorder_pending(
                    &outcome.newly_opened_gaps,
                    now_ms,
                    Some(rtp_timestamp),
                    "unknown",
                    "unknown",
                );
            }
        }
        if outcome.is_reorder {
            self.oos_event_count = self.oos_event_count.saturating_add(1);
            self.recent_oos_active_until_ms = Some(now_ms + OOS_ACTIVITY_COOLDOWN_MS);
            self.mark_frame_oos(rtp_timestamp);
            let frame_has_oos = self.frame_seen_oos(rtp_timestamp);
            let recent_oos_active = self.oos_recently_active(now_ms);
            if let Some(distance) = outcome.reorder_distance_from_highest {
                if self.recent_oos_depths.len() >= OOS_DEPTH_WINDOW_CAPACITY {
                    self.recent_oos_depths.pop_front();
                }
                self.recent_oos_depths.push_back(distance);
                self.update_dynamic_nack_skip_last_n(now_ms);
            }
            if self.oos_event_count == 1 || self.oos_event_count.is_power_of_two() {
                crate::xbx_log_info!(
                    "[RtcVideoFrameSource] oos event seq={} distance={:?} skip_last_n={}",
                    outcome.sequence,
                    outcome.reorder_distance_from_highest,
                    self.nack_skip_last_n
                );
                crate::xbx_log_info!(
                    "[RtcVideoFrameSource] oos signal frame_ts={} frame_has_oos={} recent_active={}",
                    rtp_timestamp,
                    frame_has_oos,
                    recent_oos_active
                );
            }
        }
    }

    pub(crate) fn update_dynamic_nack_skip_last_n(&mut self, now_ms: f64) {
        if self
            .last_nack_skip_last_n_updated_at_ms
            .is_some_and(|last_ms| {
                (now_ms - last_ms).max(0.0) < OOS_SKIP_LAST_N_REFRESH_INTERVAL_MS
            })
        {
            return;
        }
        self.last_nack_skip_last_n_updated_at_ms = Some(now_ms);
        if self.recent_oos_depths.is_empty() {
            self.nack_skip_last_n = 2;
            return;
        }
        let mut samples: Vec<u16> = self.recent_oos_depths.iter().copied().collect();
        samples.sort_unstable();
        let last_index = samples.len() - 1;
        let p50 = samples[(last_index / 2).min(last_index)];
        let p75 = samples[((last_index * 3) / 4).min(last_index)];
        let p90 = samples[((last_index * 9) / 10).min(last_index)];
        self.nack_skip_last_n = if p90 >= 6 {
            6
        } else if p75 >= 4 {
            4
        } else if p50 <= 2 {
            2
        } else {
            4
        };
    }

    pub(crate) fn mark_frame_oos(&mut self, rtp_timestamp: u32) {
        if let Some((_, flag)) = self
            .frame_oos_flags
            .iter_mut()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
        {
            *flag = true;
            return;
        }
        if self.frame_oos_flags.len() >= FRAME_OOS_TRACK_CAPACITY {
            self.frame_oos_flags.pop_front();
        }
        self.frame_oos_flags.push_back((rtp_timestamp, true));
    }

    pub(super) fn frame_seen_oos(&self, rtp_timestamp: u32) -> bool {
        self.frame_oos_flags
            .iter()
            .find(|(timestamp, _)| *timestamp == rtp_timestamp)
            .is_some_and(|(_, flag)| *flag)
    }

    pub(super) fn oos_recently_active(&self, now_ms: f64) -> bool {
        self.recent_oos_active_until_ms
            .is_some_and(|until_ms| now_ms <= until_ms)
    }

    pub(crate) async fn recv_frame_inner(&mut self) -> Option<AssembledVideoFrame> {
        const MAX_ACCESS_UNIT_CONTINUE_SPIN_PER_RECV: u32 = 128;
        let mut access_unit_continue_spins = 0u32;
        loop {
            self.sync_recovery_ledger_to_stats();
            self.maybe_ack_clean_anchor_commit_from_runtime_stats();
            if self.should_run_nack_maintenance_tick() {
                self.maybe_run_nack_maintenance().await;
            }
            self.maybe_emit_jitter_early_boundary();
            if let Some(sample) = self
                .receive_core_mut()
                .receive_engine
                .frame_assembler
                .pop_access_unit()
            {
                self.record_access_unit_popped();
                match DecodeGate::default()
                    .evaluate_for_ingress(self, sample)
                    .await
                {
                    DecodeGateDecision::Emit(frame) => {
                        self.record_decode_gate_emit();
                        return Some(frame);
                    }
                    DecodeGateDecision::Continue => {
                        self.record_decode_gate_continue();
                        access_unit_continue_spins = access_unit_continue_spins.saturating_add(1);
                        tokio::task::yield_now().await;
                        if self.rx.is_closed()
                            && access_unit_continue_spins >= MAX_ACCESS_UNIT_CONTINUE_SPIN_PER_RECV
                        {
                            return None;
                        }
                        if access_unit_continue_spins < MAX_ACCESS_UNIT_CONTINUE_SPIN_PER_RECV {
                            continue;
                        }
                        access_unit_continue_spins = 0;
                    }
                }
            } else {
                access_unit_continue_spins = 0;
            }

            let now = std::time::Instant::now();
            let (effective_idle_timeout, effective_idle_hint_cooldown) =
                self.resolve_effective_idle_controls();
            let idle_timeout = should_trigger_idle_timeout(
                self.received_packet_count > 0,
                now,
                self.last_packet_time,
                effective_idle_timeout,
            );
            let idle_timeout =
                idle_timeout && !self.should_absorb_idle_timeout(effective_idle_timeout);
            let thin_stream_stall = self.should_trigger_thin_stream_stall(now);
            let idle_timeout = if idle_timeout {
                self.should_emit_confirmed_idle_timeout(now, effective_idle_timeout)
            } else {
                self.pending_idle_timeout_since = None;
                false
            };
            let thin_stream_stall = if thin_stream_stall {
                self.should_emit_confirmed_thin_stream_stall(now)
            } else {
                self.pending_thin_stream_since = None;
                false
            };

            if idle_timeout || thin_stream_stall {
                let timeout_reason = if thin_stream_stall {
                    "streamThinStall"
                } else {
                    "streamIdleTimeout"
                };
                let timeout_source_event = if thin_stream_stall {
                    "timeout-stream-thin-stall"
                } else {
                    "timeout-stream-idle"
                };
                let timeout_now_ms = now_ms_f64();
                let sustaining_recovery_failed = self.should_keep_timeout_recovery_receiver_local();
                self.trace_ledger.record_timeout_reason(timeout_reason);
                self.trace_ledger.on_timeout_detected();
                self.record_video_timeline_observation(
                    timeout_source_event,
                    None,
                    None,
                    timeout_now_ms,
                );
                self.receive_core_mut()
                    .receive_engine
                    .frame_assembler
                    .reset_builder();
                self.last_packet_time = now;

                if self.last_idle_hint_time.map_or(true, |t| {
                    now.duration_since(t) >= effective_idle_hint_cooldown
                }) {
                    self.last_idle_hint_time = Some(now);
                    if sustaining_recovery_failed {
                        let should_soft_request = self
                            .runtime_stats
                            .read(|stats| {
                                Self::should_soft_request_recovery_keyframe(
                                    stats,
                                    timeout_now_ms,
                                    None,
                                    false,
                                    false,
                                    true,
                                )
                            })
                            .unwrap_or(false);
                        if should_soft_request {
                            self.request_recovery_keyframe_soft_from_source(
                                "chain-recovery-anchor-requested",
                                None,
                                timeout_now_ms,
                            );
                        } else {
                            self.request_recovery_keyframe_from_source(
                                "chain-recovery-anchor-requested",
                                None,
                                timeout_now_ms,
                            );
                        }
                    } else {
                        self.queue_transport_observation(if thin_stream_stall {
                            TransportObservation::StreamThinStall
                        } else {
                            TransportObservation::StreamIdleTimeout
                        });
                    }
                }
                if self.rx.is_closed() {
                    return None;
                }
                continue;
            }

            // 当 RTX 已经命中首洞并排进 reinject queue 时，优先给主 reader 一个很短的直接出队窗口。
            // 否则外层固定 50ms timeout 很容易一直打断普通读路径，导致 queued 包迟迟走不到 deliveredPrimary。
            let read_timeout = if self.should_prioritize_reinject_drain() {
                std::time::Duration::from_millis(8)
            } else {
                std::time::Duration::from_millis(50)
            };
            let read_timeout = self.nack_maintenance_timeout(read_timeout);
            if let Some(observation) = self
                .runtime_stats
                .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
                .flatten()
            {
                if observation.stage == "queued" && observation.pending_queue_len > 0 {
                    self.reinject_read_poll_count = self.reinject_read_poll_count.saturating_add(1);
                    if self.reinject_read_poll_count == 1
                        || self.reinject_read_poll_count.is_power_of_two()
                    {
                        crate::xbx_log_warn!(
                            "[RtcVideoFrameSource] reinjectReadPoll pending={} gap={:?} nack={:?}..{:?} timeout_ms={} count={}",
                            observation.pending_queue_len,
                            observation.matched_gap_sequence,
                            observation.matched_nack_first_sequence,
                            observation.matched_nack_last_sequence,
                            read_timeout.as_millis(),
                            self.reinject_read_poll_count
                        );
                    }
                }
            }
            match tokio::time::timeout(read_timeout, self.rx.recv()).await {
                Ok(Some(rtp_video_packet)) => {
                    self.clear_pending_timeout_confirmations();
                    self.received_packet_count = self.received_packet_count.saturating_add(1);
                    let ingress_kind = rtp_video_packet.ingress_kind;
                    let rtp = rtp_video_packet.to_rtp_packet();
                    self.last_packet_time = std::time::Instant::now();
                    self.remember_frame_playout_base_candidate(
                        rtp.header.timestamp,
                        self.last_packet_time,
                    );
                    let seq = rtp.header.sequence_number;
                    let now_ms = now_ms_f64();
                    if matches!(ingress_kind, RtcVideoIngressKind::RtxReinject { .. }) {
                        self.receive_core_mut()
                            .receive_engine
                            .mark_sequence_recovered(
                                seq,
                                crate::transport::rtc::receive::nack_requester::RecoveredPacketSource::Rtx,
                            );
                    }
                    let observe_outcome = self
                        .receive_core_mut()
                        .receive_engine
                        .observe_rtp_sequence(seq, now_ms);
                    self.on_rtp_sequence_observed(&observe_outcome, rtp.header.timestamp, now_ms);

                    // 更新帧边界追踪状态
                    let is_priority =
                        crate::transport::rtc::receive::rtx_sink::is_likely_h264_recovery_priority(
                            &rtp.payload,
                        );
                    if let Ok(mut tracker) = self.frame_boundary.lock() {
                        tracker.on_packet_arrived(
                            seq,
                            rtp.header.timestamp,
                            rtp.header.marker,
                            is_priority,
                        );
                    }

                    let reinject_observation = self.reinject_observation_for_ingress(
                        ingress_kind,
                        rtp.header.ssrc,
                        seq,
                        rtp.header.timestamp,
                        now_ms,
                    );
                    if let Some(observation) = reinject_observation.as_ref() {
                        self.runtime_stats
                            .record_video_rtx_reinject(observation.clone());
                    }
                    self.push_recent_rtp_packet(seq, rtp.header.timestamp);
                    if let Some(observation) = reinject_observation.as_ref() {
                        self.record_reinject_stage(observation, "sampleBuilderPush", now_ms);
                    }
                    if observe_outcome.resolved_pending_nack {
                        self.trace_ledger.mark_gap_resolved(
                            seq,
                            now_ms,
                            Some(rtp.header.timestamp),
                            "transport",
                            gap_transport_evidence(Some(false)),
                        );
                        self.record_video_timeline_observation(
                            "gap-resolved",
                            Some(seq),
                            Some(rtp.header.timestamp),
                            now_ms,
                        );
                        self.record_anchor_candidate_ledger(
                            Some(rtp.header.timestamp),
                            "gap-resolved",
                            XbxEngineAnchorCandidateState::Repaired,
                            None,
                            now_ms,
                        );
                        if let Some(observation) = reinject_observation.as_ref() {
                            let mut resolved_observation = observation.clone();
                            resolved_observation.matched_nack_range = true;
                            resolved_observation.matched_pending_gap = true;
                            resolved_observation.matched_gap_sequence = Some(seq);
                            resolved_observation.matched_nack_first_sequence = Some(seq);
                            resolved_observation.matched_nack_last_sequence = Some(seq);
                            self.record_reinject_stage(
                                &resolved_observation,
                                "adapterResolved",
                                now_ms,
                            );
                        }
                        self.record_receiver_local_nack_recovered(seq, now_ms, false);
                    } else if let Some(observation) = reinject_observation.as_ref() {
                        self.record_reinject_stage(observation, "adapterResolveMiss", now_ms);
                    }
                    let (next_highest_sequence, forward_gap) =
                        detect_forward_gap(self.last_highest_rtp_sequence, seq);
                    self.last_highest_rtp_sequence = next_highest_sequence;
                    if let Some((expected_sequence, received_sequence)) = forward_gap {
                        let missing_sequences =
                            crate::transport::rtc::receive::nack_maintenance::wrapping_sequence_range(
                            expected_sequence,
                            received_sequence,
                        );
                        // 前向 gap：匿名缺洞保守处理为 disposable
                        self.trace_ledger.observe_gap(
                            &missing_sequences,
                            now_ms,
                            Some(rtp.header.timestamp),
                            "disposable",
                            "unknown",
                        );
                        if let Some(sequence) = missing_sequences.first().copied() {
                            self.record_video_timeline_observation(
                                "gap-observed-forward-packet",
                                Some(sequence),
                                Some(rtp.header.timestamp),
                                now_ms,
                            );
                        }
                    }
                    self.maybe_run_nack_maintenance().await;
                    if seq % 100 == 0 {
                        crate::xbx_log_info!(
                            "[RtcVideoFrameSource] RTP packet received: seq={}, ts={}",
                            seq,
                            rtp.header.timestamp
                        );
                    }
                    if self.received_packet_count == 1
                        || self.received_packet_count.is_power_of_two()
                    {
                        crate::xbx_log_info!(
                            "[RtcVideoFrameSource] packet received count={} seq={} ts={}",
                            self.received_packet_count,
                            seq,
                            rtp.header.timestamp
                        );
                    }
                    if !matches!(ingress_kind, RtcVideoIngressKind::RtxReinject { .. }) {
                        self.current_media_ssrc = Some(rtp.header.ssrc);
                    }
                    self.record_frame_first_packet_sequence(
                        rtp.header.timestamp,
                        rtp.header.sequence_number,
                    );
                    if rtp.header.marker {
                        self.note_ingress_waiting_rtp_marker();
                        self.jitter_marker_seen_count =
                            self.jitter_marker_seen_count.saturating_add(1);
                        self.record_rtp_marker_observed();
                        self.pending_marker_boundary = Some(
                            crate::transport::rtc::receive::ingress_state::PendingMarkerBoundary {
                                sequence: rtp.header.sequence_number,
                                rtp_timestamp: rtp.header.timestamp,
                                media_payload_type: rtp.header.payload_type,
                                observed_at: std::time::Instant::now(),
                            },
                        );
                        if self.jitter_marker_seen_count == 1
                            || self.jitter_marker_seen_count.is_power_of_two()
                        {
                            crate::xbx_log_debug!(
                                "[RtcVideoFrameSource] marker observed count={} seq={} ts={} early_emit={}",
                                self.jitter_marker_seen_count,
                                rtp.header.sequence_number,
                                rtp.header.timestamp,
                                self.jitter_early_emit_enabled
                            );
                        }
                    }
                    let packet_arrived_at = self.last_packet_time;
                    self.receive_core_mut()
                        .receive_engine
                        .frame_assembler
                        .push_rtp(rtp, packet_arrived_at);
                }
                Ok(None) => {
                    while let Some(sample) = self
                        .receive_core_mut()
                        .receive_engine
                        .frame_assembler
                        .pop_access_unit()
                    {
                        self.record_access_unit_popped();
                        match DecodeGate::default()
                            .evaluate_for_ingress(self, sample)
                            .await
                        {
                            DecodeGateDecision::Emit(frame) => {
                                self.record_decode_gate_emit();
                                return Some(frame);
                            }
                            DecodeGateDecision::Continue => {
                                self.record_decode_gate_continue();
                                continue;
                            }
                        }
                    }
                    let cause = self.runtime_stats.current_video_ingress_close_cause();
                    let now_ms = now_ms_f64();
                    self.runtime_stats
                        .record_video_ingress_rx_closed(now_ms, cause.as_deref());
                    crate::xbx_log_error!(
                        "[RtcVideoFrameSource] rx closed cause={}",
                        cause.as_deref().unwrap_or("upstreamSenderDropped")
                    );
                    return None;
                }
                Err(_) => {
                    self.maybe_run_nack_maintenance().await;
                }
            }
        }
    }

    fn record_rtp_marker_observed(&self) {
        self.runtime_stats.update(|stats| {
            stats.inbound_video_rtp_marker_count_total =
                stats.inbound_video_rtp_marker_count_total.saturating_add(1);
        });
    }

    fn record_access_unit_popped(&self) {
        self.runtime_stats.update(|stats| {
            stats.inbound_video_access_unit_count_total = stats
                .inbound_video_access_unit_count_total
                .saturating_add(1);
        });
    }

    fn record_decode_gate_emit(&self) {
        self.runtime_stats.update(|stats| {
            stats.inbound_video_decode_gate_emit_count_total = stats
                .inbound_video_decode_gate_emit_count_total
                .saturating_add(1);
        });
    }

    fn record_decode_gate_continue(&self) {
        self.runtime_stats.update(|stats| {
            stats.inbound_video_decode_gate_continue_count_total = stats
                .inbound_video_decode_gate_continue_count_total
                .saturating_add(1);
        });
    }
}

pub(crate) fn should_trigger_idle_timeout(
    has_received_packet: bool,
    now: std::time::Instant,
    last_packet_time: std::time::Instant,
    idle_timeout: std::time::Duration,
) -> bool {
    // 首包到来前不把“没有媒体包”当成 idle，避免启动/握手期误触发超时诊断。
    has_received_packet && now.duration_since(last_packet_time) > idle_timeout
}

fn idle_timeout_render_slack_window_ms(idle_timeout: std::time::Duration) -> f64 {
    const IDLE_TIMEOUT_SLACK_WINDOW_MIN_MS: f64 = 220.0;
    const IDLE_TIMEOUT_SLACK_WINDOW_MAX_MS: f64 = 450.0;
    let scaled = (idle_timeout.as_millis() as f64) * 1.5;
    scaled
        .max(IDLE_TIMEOUT_SLACK_WINDOW_MIN_MS)
        .min(IDLE_TIMEOUT_SLACK_WINDOW_MAX_MS)
}

pub(crate) fn should_absorb_idle_timeout_for_steady_gap(
    transport_state: XbxEngineTransportStateDto,
    current_recovery_epoch: u64,
    clean_anchor_epoch: Option<u64>,
    clean_anchor_source_event: Option<&str>,
    latest_video_host_present_time_ms: Option<f64>,
    latest_video_decode_ok_time_ms: Option<f64>,
    video_renderer_stalled: Option<bool>,
    video_decoder_stalled: Option<bool>,
    now_ms: f64,
    idle_timeout: std::time::Duration,
) -> bool {
    if transport_state != XbxEngineTransportStateDto::Connected {
        return false;
    }
    if video_renderer_stalled.unwrap_or(false) || video_decoder_stalled.unwrap_or(false) {
        return false;
    }
    let has_current_clean_anchor = clean_anchor_epoch.is_some_and(|epoch| {
        epoch == current_recovery_epoch
            && matches!(
                clean_anchor_source_event,
                Some("decoded-usable-idr" | "clean-anchor-committed" | "displayed-idr")
            )
    });
    if !has_current_clean_anchor {
        return false;
    }
    let fresh_window_ms = idle_timeout_render_slack_window_ms(idle_timeout);
    let present_fresh = latest_video_host_present_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= fresh_window_ms);
    let decode_fresh = latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= fresh_window_ms);
    present_fresh || decode_fresh
}

fn should_relax_idle_timeout(
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    feedback_interval_ms: Option<f64>,
) -> bool {
    const SLOW_FEEDBACK_INTERVAL_THRESHOLD_MS: f64 = 350.0;
    matches!(session_target_type, Some(XbxEngineTargetTypeDto::Cloud))
        || feedback_interval_ms.is_some_and(|ms| ms >= SLOW_FEEDBACK_INTERVAL_THRESHOLD_MS)
}

pub(crate) fn resolve_effective_idle_controls(
    base_idle_timeout: std::time::Duration,
    base_idle_hint_cooldown: std::time::Duration,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    feedback_interval_ms: Option<f64>,
) -> (std::time::Duration, std::time::Duration) {
    const ADAPTIVE_IDLE_TIMEOUT_MS: u64 = 700;
    if !should_relax_idle_timeout(session_target_type, feedback_interval_ms) {
        return (base_idle_timeout, base_idle_hint_cooldown);
    }

    // 云侧或慢反馈场景放宽 idle 判定，降低“反馈慢但链路仍在推进”时的误触发。
    let effective_idle_timeout =
        base_idle_timeout.max(std::time::Duration::from_millis(ADAPTIVE_IDLE_TIMEOUT_MS));
    // hint 冷却跟随放宽，避免短时间重复上报 idle。
    let effective_idle_hint_cooldown = base_idle_hint_cooldown.max(effective_idle_timeout);
    (effective_idle_timeout, effective_idle_hint_cooldown)
}

impl FrameSource for RtcVideoFrameSource {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<AssembledVideoFrame>> + Send + 'a>>
    {
        Box::pin(async move { self.recv_frame_inner().await })
    }
}

impl TransportObservationSource for RtcVideoTransportObservationSource {
    fn recv_transport_observation<'a>(
        &'a mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<TransportObservation>> + Send + 'a>,
    > {
        Box::pin(async move { self.rx.recv().await })
    }
}
