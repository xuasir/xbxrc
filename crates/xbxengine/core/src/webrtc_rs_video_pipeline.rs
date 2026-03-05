use std::collections::{BTreeMap, VecDeque};

use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoPipelineRuntimeConfig};

const MAX_FORWARD_SEQ_DISTANCE: u16 = 1024;
const MAX_GAP_TRACK_COUNT: u16 = 64;
const MAX_MISSING_PACKET_TRACK_COUNT: usize = 256;
const PLI_PACKET_GAP_THRESHOLD: u16 = 6;
const PLI_PENDING_MISSING_THRESHOLD: usize = 24;
const PLI_MIN_INTERVAL_MS: f64 = 200.0;
const NACK_MIN_BATCH_INTERVAL_MS: f64 = 20.0;
const NACK_MAX_BATCH_SIZE: usize = 16;
const LOSS_WINDOW_1S_MS: f64 = 1_000.0;
const LOSS_WINDOW_5S_MS: f64 = 5_000.0;
const NACK_RATE_WINDOW_MS: f64 = 1_000.0;
const PLI_RATE_WINDOW_MS: f64 = 60_000.0;
const LATE_RECOVERY_TRACK_WINDOW_MS: f64 = 5_000.0;
const MAX_FINALIZED_MISSING_TRACK_COUNT: usize = 2048;
const MAX_JITTER_TIMESTAMP_DELTA_RTP_UNITS: f64 = 270_000.0;
const MAX_JITTER_SAMPLE_RTP_UNITS: f64 = 45_000.0;
const MAX_REPORTED_JITTER_MS: f64 = 250.0;
const RTT_FALLBACK_MIN_MS: f64 = 8.0;
const RTT_FALLBACK_MAX_MS: f64 = 400.0;
const RTT_FALLBACK_EWMA_ALPHA: f64 = 0.2;

const REMB_EWMA_ALPHA: f64 = 0.2;
// 稳态设定点再上调 3Mbps，给发送端更多上探空间。
const REMB_TARGET_BPS: f64 = 30_000_000.0;
const REMB_MIN_BPS: f64 = 20_000_000.0;
const REMB_MAX_BPS: f64 = 33_000_000.0;
const REMB_RECOVERY_STEP_BPS: f64 = 2_000_000.0;
const REMB_GENTLE_RECOVERY_STEP_BPS: f64 = 1_000_000.0;
const REMB_DEGRADE_FACTOR: f64 = 0.85;
const REMB_LOSS_DEGRADE_THRESHOLD: f64 = 0.05;
const REMB_LOSS_RECOVERY_THRESHOLD: f64 = 0.01;
const REMB_LOSS_PRESSURE_START_THRESHOLD: f64 = 0.03;
const REMB_RTT_DEGRADE_THRESHOLD_MS: f64 = 180.0;
const REMB_RTT_RECOVERY_THRESHOLD_MS: f64 = 100.0;
const REMB_JITTER_RECOVERY_THRESHOLD_MS: f64 = 20.0;
const REMB_STARTUP_GRACE_TICKS: u64 = 5;
const REMB_STARTUP_LOSS_DEGRADE_THRESHOLD: f64 = 0.10;
const REMB_STARTUP_LOSS_RECOVERY_THRESHOLD: f64 = 0.05;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WebRtcRsVideoPipelinePacketAction {
    pub request_pli: bool,
    pub nack_sequence_numbers: Vec<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct WebRtcRsVideoPipelineControlAction {
    pub remb_bps: Option<u32>,
    pub used_nack_recovery_rtt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WebRtcRsVideoPipelineConfig {
    pub nack_window_ms: u64,
    pub nack_retry_interval_ms: u64,
    pub nack_max_retry_count: u8,
}

impl Default for WebRtcRsVideoPipelineConfig {
    fn default() -> Self {
        Self {
            nack_window_ms: 400,
            nack_retry_interval_ms: 60,
            nack_max_retry_count: 5,
        }
    }
}

impl WebRtcRsVideoPipelineConfig {
    pub(crate) fn from_runtime_config(config: &XbxEngineVideoPipelineRuntimeConfig) -> Self {
        Self {
            nack_window_ms: config.nack_window_ms.max(1),
            nack_retry_interval_ms: config.nack_retry_interval_ms.max(1),
            nack_max_retry_count: config.nack_max_retry_count.max(1),
        }
    }
}

#[derive(Clone, Debug)]
struct MissingPacketState {
    first_missing_at_ms: f64,
    last_nack_sent_at_ms: Option<f64>,
    nack_attempts: u8,
}

#[derive(Clone, Copy, Debug)]
struct PacketLossSample {
    at_ms: f64,
    expected_packets_delta: i32,
    lost_packets_delta: i32,
}

#[derive(Clone, Copy, Debug)]
struct NackSample {
    at_ms: f64,
    packet_count: usize,
}

/**
 * 三层管线状态机：
 * - NetLoop: `on_rtp_packet`
 * - StatsLoop: `on_stats_tick`
 * - ControlLoop: `on_control_tick`
 */
#[derive(Clone, Debug)]
pub(crate) struct WebRtcRsVideoPipelineState {
    config: WebRtcRsVideoPipelineConfig,
    forced_remb_kbps: Option<u32>,
    adaptive_remb_enabled: bool,
    current_remb_bps: f64,
    last_sequence: Option<u16>,
    last_packet_arrival_time_ms: Option<f64>,
    last_pli_requested_at_ms: Option<f64>,
    last_nack_batch_sent_at_ms: Option<f64>,
    last_rtp_timestamp: Option<u32>,
    last_arrival_ms_for_jitter: Option<f64>,
    jitter_rtp_units: f64,
    inbound_video_packet_count_total: u64,
    inbound_video_packet_loss_estimate_total: u64,
    inbound_video_loss_ratio_1s: f64,
    inbound_video_loss_ratio_5s: f64,
    inbound_video_jitter_ms: Option<f64>,
    video_nack_request_count_total: u64,
    video_nack_batch_count_total: u64,
    video_pli_request_count_total: u64,
    video_nack_per_sec: f64,
    video_pli_per_min: f64,
    video_pending_missing_packets: usize,
    video_loss_finalized_count_total: u64,
    video_loss_recovered_count_total: u64,
    video_loss_late_recovered_count_total: u64,
    video_nack_recovery_rtt_ms: Option<f64>,
    video_rtt_ms: Option<f64>,
    control_tick_count: u64,
    missing_packets: BTreeMap<u16, MissingPacketState>,
    finalized_missing_packets_recent: BTreeMap<u16, f64>,
    loss_samples: VecDeque<PacketLossSample>,
    nack_samples: VecDeque<NackSample>,
    pli_samples: VecDeque<f64>,
}

impl Default for WebRtcRsVideoPipelineState {
    fn default() -> Self {
        Self::new(None, true, WebRtcRsVideoPipelineConfig::default())
    }
}

impl WebRtcRsVideoPipelineState {
    pub(crate) fn new(
        forced_remb_kbps: Option<u32>,
        adaptive_remb_enabled: bool,
        config: WebRtcRsVideoPipelineConfig,
    ) -> Self {
        let current_remb_bps = forced_remb_kbps
            .map(|value| value as f64 * 1000.0)
            .unwrap_or(REMB_TARGET_BPS);
        Self {
            config,
            forced_remb_kbps,
            adaptive_remb_enabled,
            current_remb_bps,
            last_sequence: None,
            last_packet_arrival_time_ms: None,
            last_pli_requested_at_ms: None,
            last_nack_batch_sent_at_ms: None,
            last_rtp_timestamp: None,
            last_arrival_ms_for_jitter: None,
            jitter_rtp_units: 0.0,
            inbound_video_packet_count_total: 0,
            inbound_video_packet_loss_estimate_total: 0,
            inbound_video_loss_ratio_1s: 0.0,
            inbound_video_loss_ratio_5s: 0.0,
            inbound_video_jitter_ms: None,
            video_nack_request_count_total: 0,
            video_nack_batch_count_total: 0,
            video_pli_request_count_total: 0,
            video_nack_per_sec: 0.0,
            video_pli_per_min: 0.0,
            video_pending_missing_packets: 0,
            video_loss_finalized_count_total: 0,
            video_loss_recovered_count_total: 0,
            video_loss_late_recovered_count_total: 0,
            video_nack_recovery_rtt_ms: None,
            video_rtt_ms: None,
            control_tick_count: 0,
            missing_packets: BTreeMap::new(),
            finalized_missing_packets_recent: BTreeMap::new(),
            loss_samples: VecDeque::new(),
            nack_samples: VecDeque::new(),
            pli_samples: VecDeque::new(),
        }
    }

    pub(crate) fn on_rtp_packet(
        &mut self,
        sequence: u16,
        rtp_timestamp: u32,
        now_ms: f64,
    ) -> WebRtcRsVideoPipelinePacketAction {
        self.inbound_video_packet_count_total =
            self.inbound_video_packet_count_total.saturating_add(1);
        self.last_packet_arrival_time_ms = Some(now_ms);
        self.update_jitter(rtp_timestamp, now_ms);
        self.cleanup_finalized_missing_packets_recent(now_ms);

        let previous_sequence = self.last_sequence;
        let packet_gap = estimate_packet_gap(previous_sequence, sequence);
        self.last_sequence = Some(sequence);

        let expected_packets_delta = i32::from(packet_gap) + 1;
        let lost_packets_delta = i32::from(packet_gap);
        self.push_loss_sample(now_ms, expected_packets_delta, lost_packets_delta);
        self.track_missing_packets(previous_sequence, now_ms, packet_gap);
        let late_recovered = self
            .finalized_missing_packets_recent
            .remove(&sequence)
            .is_some();
        if late_recovered {
            self.video_loss_recovered_count_total =
                self.video_loss_recovered_count_total.saturating_add(1);
            self.video_loss_late_recovered_count_total =
                self.video_loss_late_recovered_count_total.saturating_add(1);
            // 即时 loss 口径：晚到恢复后回冲一笔 provisional loss。
            self.push_loss_sample(now_ms, 0, -1);
        }
        if let Some(recovered_state) = self.missing_packets.remove(&sequence) {
            self.video_loss_recovered_count_total =
                self.video_loss_recovered_count_total.saturating_add(1);
            self.update_rtt_from_nack_recovery(now_ms, &recovered_state);
            // 即时 loss 口径：NACK 后恢复的包回冲 provisional loss。
            self.push_loss_sample(now_ms, 0, -1);
        }

        let nack_sequence_numbers = self.collect_nack_batch(now_ms);
        if !nack_sequence_numbers.is_empty() {
            self.video_nack_batch_count_total = self.video_nack_batch_count_total.saturating_add(1);
            self.video_nack_request_count_total = self
                .video_nack_request_count_total
                .saturating_add(nack_sequence_numbers.len() as u64);
            self.nack_samples.push_back(NackSample {
                at_ms: now_ms,
                packet_count: nack_sequence_numbers.len(),
            });
        }

        let finalized_loss_packets = self.cleanup_missing_packets(now_ms);
        self.finalize_lost_packets(now_ms, finalized_loss_packets);
        self.video_pending_missing_packets = self.missing_packets.len();

        let severe_missing_packets = self.missing_packets.len() >= PLI_PENDING_MISSING_THRESHOLD;
        let should_request_pli = (packet_gap >= PLI_PACKET_GAP_THRESHOLD || severe_missing_packets)
            && self
                .last_pli_requested_at_ms
                .map(|last| now_ms - last >= PLI_MIN_INTERVAL_MS)
                .unwrap_or(true);
        if should_request_pli {
            self.last_pli_requested_at_ms = Some(now_ms);
            self.video_pli_request_count_total =
                self.video_pli_request_count_total.saturating_add(1);
            self.pli_samples.push_back(now_ms);
        }

        WebRtcRsVideoPipelinePacketAction {
            request_pli: should_request_pli,
            nack_sequence_numbers,
        }
    }

    pub(crate) fn on_recovered_sequence(&mut self, sequence: u16, now_ms: f64) {
        self.cleanup_finalized_missing_packets_recent(now_ms);
        let late_recovered = self
            .finalized_missing_packets_recent
            .remove(&sequence)
            .is_some();
        if late_recovered {
            self.video_loss_recovered_count_total =
                self.video_loss_recovered_count_total.saturating_add(1);
            self.video_loss_late_recovered_count_total =
                self.video_loss_late_recovered_count_total.saturating_add(1);
            // RTX 等非主轨恢复到原包序号后，回冲一笔 provisional loss。
            self.push_loss_sample(now_ms, 0, -1);
        }
        if let Some(recovered_state) = self.missing_packets.remove(&sequence) {
            self.video_loss_recovered_count_total =
                self.video_loss_recovered_count_total.saturating_add(1);
            self.update_rtt_from_nack_recovery(now_ms, &recovered_state);
            self.push_loss_sample(now_ms, 0, -1);
        }
        self.video_pending_missing_packets = self.missing_packets.len();
    }

    pub(crate) fn on_stats_tick(&mut self, now_ms: f64) {
        self.cleanup_finalized_missing_packets_recent(now_ms);
        let finalized_loss_packets = self.cleanup_missing_packets(now_ms);
        self.finalize_lost_packets(now_ms, finalized_loss_packets);
        self.prune_loss_samples(now_ms);
        self.prune_nack_samples(now_ms);
        self.prune_pli_samples(now_ms);

        self.inbound_video_loss_ratio_1s = self.compute_loss_ratio(LOSS_WINDOW_1S_MS, now_ms);
        self.inbound_video_loss_ratio_5s = self.compute_loss_ratio(LOSS_WINDOW_5S_MS, now_ms);
        self.video_nack_per_sec = self.compute_nack_per_sec(now_ms);
        self.video_pli_per_min = self.compute_pli_per_min(now_ms);
        self.video_pending_missing_packets = self.missing_packets.len();
    }

    pub(crate) fn on_control_tick(
        &mut self,
        rtt_ms: Option<f64>,
    ) -> WebRtcRsVideoPipelineControlAction {
        self.control_tick_count = self.control_tick_count.saturating_add(1);
        let stats_rtt_ms = rtt_ms.filter(|value| value.is_finite() && *value > 0.0);
        let fallback_rtt_ms = self
            .video_nack_recovery_rtt_ms
            .filter(|value| value.is_finite() && *value > 0.0);
        let effective_rtt_ms = stats_rtt_ms.or(fallback_rtt_ms);
        if let Some(rtt_ms) = effective_rtt_ms {
            self.video_rtt_ms = Some(rtt_ms);
        }
        let used_nack_recovery_rtt = stats_rtt_ms.is_none() && fallback_rtt_ms.is_some();

        if let Some(forced_remb_kbps) = self.forced_remb_kbps {
            self.current_remb_bps = (forced_remb_kbps as f64) * 1000.0;
            return WebRtcRsVideoPipelineControlAction {
                remb_bps: Some(self.current_remb_bps.round().clamp(0.0, u32::MAX as f64) as u32),
                used_nack_recovery_rtt,
            };
        }

        if !self.adaptive_remb_enabled {
            return WebRtcRsVideoPipelineControlAction {
                remb_bps: None,
                used_nack_recovery_rtt,
            };
        }

        let startup_grace = self.control_tick_count <= REMB_STARTUP_GRACE_TICKS;
        let target_bps = decide_remb_target_bps(
            self.inbound_video_loss_ratio_1s,
            self.inbound_video_jitter_ms,
            self.video_rtt_ms,
            self.current_remb_bps,
            startup_grace,
        );
        self.current_remb_bps = smooth_remb_bps(self.current_remb_bps, target_bps);

        WebRtcRsVideoPipelineControlAction {
            remb_bps: Some(self.current_remb_bps.round().clamp(0.0, u32::MAX as f64) as u32),
            used_nack_recovery_rtt,
        }
    }

    pub(crate) fn write_runtime_stats(&self, stats: &mut XbxEngineMediaRuntimeStats) {
        stats.latest_video_packet_arrival_time_ms = self.last_packet_arrival_time_ms;
        stats.latest_video_packet_sequence = self.last_sequence;
        stats.inbound_video_packet_count_total = self.inbound_video_packet_count_total;
        stats.inbound_video_packet_loss_estimate_total =
            self.inbound_video_packet_loss_estimate_total;
        stats.inbound_video_loss_ratio_1s = self.inbound_video_loss_ratio_1s;
        stats.inbound_video_loss_ratio_5s = self.inbound_video_loss_ratio_5s;
        stats.inbound_video_jitter_ms = self.inbound_video_jitter_ms;
        stats.video_nack_request_count_total = self.video_nack_request_count_total;
        stats.video_nack_batch_count_total = self.video_nack_batch_count_total;
        stats.video_pli_request_count_total = self.video_pli_request_count_total;
        stats.video_nack_per_sec = self.video_nack_per_sec;
        stats.video_pli_per_min = self.video_pli_per_min;
        stats.video_pending_missing_packets = self.video_pending_missing_packets;
        stats.video_loss_finalized_count_total = self.video_loss_finalized_count_total;
        stats.video_loss_recovered_count_total = self.video_loss_recovered_count_total;
        stats.video_loss_late_recovered_count_total = self.video_loss_late_recovered_count_total;
        stats.video_nack_recovery_rtt_ms = self.video_nack_recovery_rtt_ms;
        stats.video_rtt_ms = self.video_rtt_ms;
        stats.video_remb_bps =
            Some(self.current_remb_bps.round().clamp(0.0, u32::MAX as f64) as u32);
    }

    fn update_jitter(&mut self, rtp_timestamp: u32, arrival_ms: f64) {
        let Some(previous_rtp_timestamp) = self.last_rtp_timestamp else {
            self.last_rtp_timestamp = Some(rtp_timestamp);
            self.last_arrival_ms_for_jitter = Some(arrival_ms);
            return;
        };
        let Some(previous_arrival_ms) = self.last_arrival_ms_for_jitter else {
            self.last_rtp_timestamp = Some(rtp_timestamp);
            self.last_arrival_ms_for_jitter = Some(arrival_ms);
            return;
        };

        let arrival_rtp_units = arrival_ms * 90.0;
        let previous_arrival_rtp_units = previous_arrival_ms * 90.0;
        let arrival_delta = arrival_rtp_units - previous_arrival_rtp_units;
        let timestamp_delta = (rtp_timestamp.wrapping_sub(previous_rtp_timestamp)) as f64;
        if timestamp_delta <= 0.0 || timestamp_delta > MAX_JITTER_TIMESTAMP_DELTA_RTP_UNITS {
            self.reset_jitter_baseline(rtp_timestamp, arrival_ms);
            return;
        }

        let d = (arrival_delta - timestamp_delta)
            .abs()
            .clamp(0.0, MAX_JITTER_SAMPLE_RTP_UNITS);
        if !d.is_finite() {
            self.reset_jitter_baseline(rtp_timestamp, arrival_ms);
            return;
        }

        self.jitter_rtp_units += (d - self.jitter_rtp_units) / 16.0;
        self.inbound_video_jitter_ms =
            Some((self.jitter_rtp_units / 90.0).clamp(0.0, MAX_REPORTED_JITTER_MS));
        self.last_rtp_timestamp = Some(rtp_timestamp);
        self.last_arrival_ms_for_jitter = Some(arrival_ms);
    }

    fn reset_jitter_baseline(&mut self, rtp_timestamp: u32, arrival_ms: f64) {
        self.last_rtp_timestamp = Some(rtp_timestamp);
        self.last_arrival_ms_for_jitter = Some(arrival_ms);
        self.jitter_rtp_units = 0.0;
        self.inbound_video_jitter_ms = None;
    }

    fn track_missing_packets(
        &mut self,
        previous_sequence: Option<u16>,
        now_ms: f64,
        packet_gap: u16,
    ) {
        if packet_gap == 0 {
            return;
        }
        let Some(previous_sequence) = previous_sequence else {
            return;
        };

        let max_track_gap = packet_gap.min(MAX_GAP_TRACK_COUNT);
        for offset in 1..=max_track_gap {
            if self.missing_packets.len() >= MAX_MISSING_PACKET_TRACK_COUNT {
                break;
            }
            let missing_sequence = previous_sequence.wrapping_add(offset);
            self.missing_packets
                .entry(missing_sequence)
                .or_insert(MissingPacketState {
                    first_missing_at_ms: now_ms,
                    last_nack_sent_at_ms: None,
                    nack_attempts: 0,
                });
        }
    }

    fn update_rtt_from_nack_recovery(&mut self, now_ms: f64, recovered_state: &MissingPacketState) {
        let Some(last_nack_sent_at_ms) = recovered_state.last_nack_sent_at_ms else {
            return;
        };
        let sample_ms = now_ms - last_nack_sent_at_ms;
        if !sample_ms.is_finite()
            || sample_ms < RTT_FALLBACK_MIN_MS
            || sample_ms > RTT_FALLBACK_MAX_MS
        {
            return;
        }
        let next_rtt_ms = self
            .video_nack_recovery_rtt_ms
            .map(|current| {
                current * (1.0 - RTT_FALLBACK_EWMA_ALPHA) + sample_ms * RTT_FALLBACK_EWMA_ALPHA
            })
            .unwrap_or(sample_ms);
        self.video_nack_recovery_rtt_ms = Some(next_rtt_ms);
    }

    fn push_loss_sample(
        &mut self,
        at_ms: f64,
        expected_packets_delta: i32,
        lost_packets_delta: i32,
    ) {
        self.loss_samples.push_back(PacketLossSample {
            at_ms,
            expected_packets_delta,
            lost_packets_delta,
        });
    }

    fn collect_nack_batch(&mut self, now_ms: f64) -> Vec<u16> {
        let can_send_batch = self
            .last_nack_batch_sent_at_ms
            .map(|last| now_ms - last >= NACK_MIN_BATCH_INTERVAL_MS)
            .unwrap_or(true);
        if !can_send_batch {
            return Vec::new();
        }

        let mut selected_sequences = Vec::with_capacity(NACK_MAX_BATCH_SIZE);
        for (&sequence, state) in &self.missing_packets {
            if selected_sequences.len() >= NACK_MAX_BATCH_SIZE {
                break;
            }
            if now_ms - state.first_missing_at_ms > self.config.nack_window_ms as f64 {
                continue;
            }
            if state.nack_attempts >= self.config.nack_max_retry_count {
                continue;
            }
            let should_retry = state
                .last_nack_sent_at_ms
                .map(|last| now_ms - last >= self.config.nack_retry_interval_ms as f64)
                .unwrap_or(true);
            if should_retry {
                selected_sequences.push(sequence);
            }
        }

        if selected_sequences.is_empty() {
            return selected_sequences;
        }

        self.last_nack_batch_sent_at_ms = Some(now_ms);
        for sequence in &selected_sequences {
            if let Some(state) = self.missing_packets.get_mut(sequence) {
                state.nack_attempts = state.nack_attempts.saturating_add(1);
                state.last_nack_sent_at_ms = Some(now_ms);
            }
        }
        selected_sequences
    }

    fn cleanup_missing_packets(&mut self, now_ms: f64) -> Vec<u16> {
        let mut finalized_sequences = Vec::new();
        for (&sequence, state) in &self.missing_packets {
            let exceeded_window =
                now_ms - state.first_missing_at_ms > self.config.nack_window_ms as f64;
            let exhausted_retry = state.nack_attempts >= self.config.nack_max_retry_count
                && state
                    .last_nack_sent_at_ms
                    .map(|last| now_ms - last >= self.config.nack_retry_interval_ms as f64)
                    .unwrap_or(false);
            if exceeded_window || exhausted_retry {
                finalized_sequences.push(sequence);
            }
        }
        for sequence in &finalized_sequences {
            self.missing_packets.remove(sequence);
        }
        finalized_sequences
    }

    fn finalize_lost_packets(&mut self, now_ms: f64, finalized_sequences: Vec<u16>) {
        if finalized_sequences.is_empty() {
            return;
        }
        let lost_packets = finalized_sequences.len() as u16;
        self.inbound_video_packet_loss_estimate_total = self
            .inbound_video_packet_loss_estimate_total
            .saturating_add(lost_packets as u64);
        self.video_loss_finalized_count_total = self
            .video_loss_finalized_count_total
            .saturating_add(finalized_sequences.len() as u64);
        if self.finalized_missing_packets_recent.len() >= MAX_FINALIZED_MISSING_TRACK_COUNT {
            self.finalized_missing_packets_recent.clear();
        }
        for sequence in finalized_sequences {
            self.finalized_missing_packets_recent
                .insert(sequence, now_ms);
        }
    }

    fn cleanup_finalized_missing_packets_recent(&mut self, now_ms: f64) {
        self.finalized_missing_packets_recent
            .retain(|_, finalized_at_ms| {
                now_ms - *finalized_at_ms <= LATE_RECOVERY_TRACK_WINDOW_MS
            });
    }

    fn prune_loss_samples(&mut self, now_ms: f64) {
        while self
            .loss_samples
            .front()
            .is_some_and(|sample| now_ms - sample.at_ms > LOSS_WINDOW_5S_MS)
        {
            let _ = self.loss_samples.pop_front();
        }
    }

    fn prune_nack_samples(&mut self, now_ms: f64) {
        while self
            .nack_samples
            .front()
            .is_some_and(|sample| now_ms - sample.at_ms > NACK_RATE_WINDOW_MS)
        {
            let _ = self.nack_samples.pop_front();
        }
    }

    fn prune_pli_samples(&mut self, now_ms: f64) {
        while self
            .pli_samples
            .front()
            .is_some_and(|at_ms| now_ms - *at_ms > PLI_RATE_WINDOW_MS)
        {
            let _ = self.pli_samples.pop_front();
        }
    }

    fn compute_loss_ratio(&self, window_ms: f64, now_ms: f64) -> f64 {
        let mut lost_packets = 0i64;
        let mut expected_packets = 0i64;

        for sample in self.loss_samples.iter().rev() {
            if now_ms - sample.at_ms > window_ms {
                break;
            }
            lost_packets = lost_packets.saturating_add(i64::from(sample.lost_packets_delta));
            expected_packets =
                expected_packets.saturating_add(i64::from(sample.expected_packets_delta));
        }

        if expected_packets <= 0 {
            return 0.0;
        }
        (lost_packets.max(0) as f64 / expected_packets as f64).clamp(0.0, 1.0)
    }

    fn compute_nack_per_sec(&self, now_ms: f64) -> f64 {
        let mut packet_count = 0usize;
        for sample in self.nack_samples.iter().rev() {
            if now_ms - sample.at_ms > NACK_RATE_WINDOW_MS {
                break;
            }
            packet_count = packet_count.saturating_add(sample.packet_count);
        }
        packet_count as f64
    }

    fn compute_pli_per_min(&self, now_ms: f64) -> f64 {
        let mut event_count = 0usize;
        for at_ms in self.pli_samples.iter().rev() {
            if now_ms - *at_ms > PLI_RATE_WINDOW_MS {
                break;
            }
            event_count = event_count.saturating_add(1);
        }
        event_count as f64
    }
}

fn decide_remb_target_bps(
    loss_ratio_1s: f64,
    jitter_ms: Option<f64>,
    rtt_ms: Option<f64>,
    current_remb_bps: f64,
    startup_grace: bool,
) -> f64 {
    let loss_degrade_threshold = if startup_grace {
        REMB_STARTUP_LOSS_DEGRADE_THRESHOLD
    } else {
        REMB_LOSS_DEGRADE_THRESHOLD
    };
    let loss_recovery_threshold = if startup_grace {
        REMB_STARTUP_LOSS_RECOVERY_THRESHOLD
    } else {
        REMB_LOSS_RECOVERY_THRESHOLD
    };
    let rtt_penalty = rtt_ms
        .map(|value| ((value - REMB_RTT_RECOVERY_THRESHOLD_MS) / 140.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    // 2~3% 丢包在云游戏公网链路较常见，不应直接触发持续下压。
    let loss_penalty =
        ((loss_ratio_1s - REMB_LOSS_PRESSURE_START_THRESHOLD) / 0.05).clamp(0.0, 1.0);
    let pressure = loss_penalty.max(rtt_penalty);

    if loss_ratio_1s > loss_degrade_threshold
        || rtt_ms
            .map(|value| value > REMB_RTT_DEGRADE_THRESHOLD_MS)
            .unwrap_or(false)
    {
        // 严重恶化时快速回撤。
        return (current_remb_bps * REMB_DEGRADE_FACTOR).max(REMB_MIN_BPS);
    }

    if pressure > 0.0 {
        // 轻中度恶化时温和回撤，避免在 20Mbps 附近过快锁死。
        let step = 0.015 + pressure * 0.055;
        return (current_remb_bps * (1.0 - step)).clamp(REMB_MIN_BPS, REMB_MAX_BPS);
    }

    let recovered = loss_ratio_1s < loss_recovery_threshold
        && rtt_ms
            .map(|value| value < REMB_RTT_RECOVERY_THRESHOLD_MS)
            .unwrap_or(true)
        && jitter_ms
            .map(|value| value < REMB_JITTER_RECOVERY_THRESHOLD_MS)
            .unwrap_or(true);
    if recovered {
        return (current_remb_bps + REMB_RECOVERY_STEP_BPS).min(REMB_TARGET_BPS);
    }

    // 稳态非严重恶化时，给一个慢速回升，帮助码率重新探测到高位。
    let can_gently_recover = loss_ratio_1s < 0.04
        && rtt_ms
            .map(|value| value < REMB_RTT_DEGRADE_THRESHOLD_MS)
            .unwrap_or(true);
    if can_gently_recover {
        return (current_remb_bps + REMB_GENTLE_RECOVERY_STEP_BPS).min(REMB_TARGET_BPS);
    }

    current_remb_bps.clamp(REMB_MIN_BPS, REMB_MAX_BPS)
}

fn smooth_remb_bps(current_bps: f64, target_bps: f64) -> f64 {
    let smoothed = current_bps * (1.0 - REMB_EWMA_ALPHA) + target_bps * REMB_EWMA_ALPHA;
    smoothed.clamp(REMB_MIN_BPS, REMB_MAX_BPS)
}

fn estimate_packet_gap(previous: Option<u16>, current: u16) -> u16 {
    let Some(previous) = previous else {
        return 0;
    };

    let forward_distance = current.wrapping_sub(previous);
    if forward_distance <= 1 {
        return 0;
    }
    if forward_distance > MAX_FORWARD_SEQ_DISTANCE {
        return 0;
    }
    forward_distance.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{WebRtcRsVideoPipelineConfig, WebRtcRsVideoPipelineState};
    use crate::XbxEngineMediaRuntimeStats;

    #[test]
    fn no_gap_on_first_packet() {
        let mut pipeline = WebRtcRsVideoPipelineState::default();
        let action = pipeline.on_rtp_packet(100, 9000, 1_000.0);
        assert!(!action.request_pli);
        assert!(action.nack_sequence_numbers.is_empty());

        pipeline.on_stats_tick(1_100.0);
        let mut stats = XbxEngineMediaRuntimeStats::default();
        pipeline.write_runtime_stats(&mut stats);
        assert_eq!(stats.inbound_video_packet_count_total, 1);
        assert_eq!(stats.inbound_video_packet_loss_estimate_total, 0);
    }

    #[test]
    fn large_gap_requests_pli_and_nack_batch() {
        let mut pipeline = WebRtcRsVideoPipelineState::default();
        let _ = pipeline.on_rtp_packet(100, 9000, 1_000.0);
        let action = pipeline.on_rtp_packet(108, 9720, 1_100.0);
        assert!(action.request_pli);
        assert_eq!(
            action.nack_sequence_numbers,
            vec![101, 102, 103, 104, 105, 106, 107]
        );

        pipeline.on_stats_tick(1_600.0);
        let mut stats = XbxEngineMediaRuntimeStats::default();
        pipeline.write_runtime_stats(&mut stats);
        assert_eq!(stats.inbound_video_packet_loss_estimate_total, 7);
        assert_eq!(stats.video_nack_request_count_total, 7);
        assert_eq!(stats.video_nack_batch_count_total, 1);
        assert!(stats.video_nack_per_sec > 0.0);
        assert!(stats.video_pli_per_min > 0.0);
    }

    #[test]
    fn nack_batch_is_throttled() {
        let mut pipeline = WebRtcRsVideoPipelineState::default();
        let _ = pipeline.on_rtp_packet(100, 9000, 1_000.0);
        let _ = pipeline.on_rtp_packet(108, 9720, 1_100.0);
        let action = pipeline.on_rtp_packet(116, 10440, 1_110.0);
        assert!(action.nack_sequence_numbers.is_empty());

        let action_after_cooldown = pipeline.on_rtp_packet(124, 11160, 1_140.0);
        assert!(!action_after_cooldown.nack_sequence_numbers.is_empty());
    }

    #[test]
    fn loss_ratio_windows_update() {
        let mut pipeline = WebRtcRsVideoPipelineState::default();
        let _ = pipeline.on_rtp_packet(100, 9000, 1_000.0);
        let _ = pipeline.on_rtp_packet(108, 9720, 1_100.0);
        let _ = pipeline.on_rtp_packet(109, 9810, 1_200.0);

        pipeline.on_stats_tick(1_600.0);
        let mut stats = XbxEngineMediaRuntimeStats::default();
        pipeline.write_runtime_stats(&mut stats);
        assert!(stats.inbound_video_loss_ratio_1s > 0.0);
        assert!(stats.inbound_video_loss_ratio_5s > 0.0);
    }

    #[test]
    fn control_tick_keeps_last_valid_rtt_sample() {
        let mut pipeline =
            WebRtcRsVideoPipelineState::new(None, true, WebRtcRsVideoPipelineConfig::default());
        let _ = pipeline.on_control_tick(Some(120.0));
        let _ = pipeline.on_control_tick(None);

        let mut stats = XbxEngineMediaRuntimeStats::default();
        pipeline.write_runtime_stats(&mut stats);
        assert_eq!(stats.video_rtt_ms, Some(120.0));
    }

    #[test]
    fn jitter_resets_when_timestamp_delta_is_abnormal() {
        let mut pipeline = WebRtcRsVideoPipelineState::default();
        let _ = pipeline.on_rtp_packet(100, 9_000, 1_000.0);
        let _ = pipeline.on_rtp_packet(101, 9_090, 1_016.0);
        let _ = pipeline.on_rtp_packet(102, 2_000_000, 1_032.0);

        let mut stats = XbxEngineMediaRuntimeStats::default();
        pipeline.write_runtime_stats(&mut stats);
        assert!(stats.inbound_video_jitter_ms.unwrap_or_default() <= super::MAX_REPORTED_JITTER_MS);
    }

    #[test]
    fn adaptive_remb_changes_with_loss_and_rtt() {
        let mut pipeline =
            WebRtcRsVideoPipelineState::new(None, true, WebRtcRsVideoPipelineConfig::default());
        let _ = pipeline.on_rtp_packet(100, 9000, 1_000.0);
        let _ = pipeline.on_rtp_packet(120, 10800, 1_100.0);
        pipeline.on_stats_tick(1_200.0);

        let degraded = pipeline.on_control_tick(Some(220.0));
        assert!(degraded.remb_bps.is_some());

        let _ = pipeline.on_rtp_packet(121, 10890, 2_200.0);
        let _ = pipeline.on_rtp_packet(122, 10980, 2_300.0);
        pipeline.on_stats_tick(2_400.0);
        let recovered = pipeline.on_control_tick(Some(80.0));
        assert!(recovered.remb_bps.is_some());
    }

    #[test]
    fn finalized_loss_counter_increases_after_missing_packets_timeout() {
        let mut pipeline = WebRtcRsVideoPipelineState::default();
        let _ = pipeline.on_rtp_packet(100, 9_000, 1_000.0);
        let _ = pipeline.on_rtp_packet(108, 9_720, 1_100.0);
        pipeline.on_stats_tick(1_600.0);

        let mut stats = XbxEngineMediaRuntimeStats::default();
        pipeline.write_runtime_stats(&mut stats);
        assert_eq!(stats.video_loss_finalized_count_total, 7);
        assert_eq!(stats.video_loss_late_recovered_count_total, 0);
    }

    #[test]
    fn late_recovered_counter_increases_when_finalized_packet_arrives_late() {
        let mut pipeline = WebRtcRsVideoPipelineState::default();
        let _ = pipeline.on_rtp_packet(100, 9_000, 1_000.0);
        let _ = pipeline.on_rtp_packet(108, 9_720, 1_100.0);
        pipeline.on_stats_tick(1_600.0);

        let _ = pipeline.on_rtp_packet(103, 9_990, 1_720.0);

        let mut stats = XbxEngineMediaRuntimeStats::default();
        pipeline.write_runtime_stats(&mut stats);
        assert_eq!(stats.video_loss_finalized_count_total, 7);
        assert_eq!(stats.video_loss_late_recovered_count_total, 1);
    }
}
