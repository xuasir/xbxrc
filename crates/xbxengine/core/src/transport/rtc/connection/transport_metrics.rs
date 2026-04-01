use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::{collections::HashMap, fmt::Write as _};

use rtc::peer_connection::transport::RTCIceCandidateType;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use rtc::statistics::report::{RTCStatsReport, RTCStatsReportEntry};
use rtc::statistics::StatsSelector;
use rtc_rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::builder::ControlledPeerConnection;
use crate::transport::rtc::events::RtcConnectionLifecycleState;
use crate::transport::rtc::facts::{PeerFact, TransportFact};
use crate::transport::rtc::stats::now_ms_f64;
use crate::{
    XbxEngineMediaRuntimeStats, XbxEngineTwccObservationQuality, XbxEngineVideoTwccObservation,
};

pub(crate) const TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK: &str = "local-feedback";
pub(crate) const TWCC_OBSERVATION_SOURCE_REMOTE_RTCP: &str = "remote-rtcp";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RtcTransportMetricsSnapshot {
    pub(crate) video_rtt_ms: Option<f64>,
    pub(crate) video_rtt_source: Option<String>,
    pub(crate) inbound_video_loss_ratio_5s: f64,
    pub(crate) inbound_video_loss_ratio_1s: f64,
    pub(crate) transport_path: Option<String>,
    pub(crate) transport_candidate_pair: Option<String>,
    pub(crate) transport_protocol: Option<String>,
    pub(crate) transport_address_family: Option<String>,
    pub(crate) inbound_video_bitrate_kbps: f64,
    pub(crate) inbound_primary_video_bytes_total: u64,
}

pub(crate) fn collect_transport_metrics(
    peer_connection: &mut ControlledPeerConnection,
    connected_at_ms: Option<f64>,
    previous_sample_at_ms: Option<f64>,
    previous_inbound_video_bytes_total: Option<u64>,
) -> Option<RtcTransportMetricsSnapshot> {
    let report = peer_connection.get_stats(Instant::now(), StatsSelector::None);
    collect_transport_metrics_from_report(
        &report,
        connected_at_ms,
        previous_sample_at_ms,
        previous_inbound_video_bytes_total,
    )
}

pub(crate) fn describe_selected_candidate_pair(
    peer_connection: &mut ControlledPeerConnection,
) -> Option<String> {
    let report = peer_connection.get_stats(Instant::now(), StatsSelector::None);
    let pair = selected_candidate_pair(&report)?;
    let local = candidate_summary(&report, &pair.local_candidate_id);
    let remote = candidate_summary(&report, &pair.remote_candidate_id);
    Some(format!(
        "state={:?} nominated={} rtt_ms={:.1} requests_sent={} responses_received={} local={} remote={}",
        pair.state,
        pair.nominated,
        pair.current_round_trip_time * 1_000.0,
        pair.requests_sent,
        pair.responses_received,
        local,
        remote,
    ))
}

fn collect_transport_metrics_from_report(
    report: &RTCStatsReport,
    connected_at_ms: Option<f64>,
    previous_sample_at_ms: Option<f64>,
    previous_inbound_video_bytes_total: Option<u64>,
) -> Option<RtcTransportMetricsSnapshot> {
    let now_ms = now_ms_f64();
    let selected_pair = selected_candidate_pair(report)?;
    let local_candidate_type = candidate_type_for(report, &selected_pair.local_candidate_id);
    let remote_candidate_type = candidate_type_for(report, &selected_pair.remote_candidate_id);
    let (video_rtt_ms, video_rtt_source) = select_video_rtt(report, selected_pair);
    let transport_path = classify_transport_path(local_candidate_type, remote_candidate_type);
    let transport_candidate_pair =
        build_transport_candidate_pair(local_candidate_type, remote_candidate_type);
    let transport_protocol = resolve_transport_protocol(
        candidate_protocol_for(report, &selected_pair.local_candidate_id),
        candidate_protocol_for(report, &selected_pair.remote_candidate_id),
    );
    let transport_address_family = Some(
        resolve_transport_address_family(
            candidate_address_family_for(report, &selected_pair.local_candidate_id),
            candidate_address_family_for(report, &selected_pair.remote_candidate_id),
        )
        .to_string(),
    );

    let (inbound_video_loss_ratio_5s, inbound_video_loss_ratio_1s, inbound_video_bytes_total) =
        select_video_inbound_metrics(report);
    let inbound_video_bitrate_kbps = estimate_recent_inbound_bitrate_kbps(
        inbound_video_bytes_total,
        previous_inbound_video_bytes_total,
        connected_at_ms,
        previous_sample_at_ms,
        now_ms,
    );

    Some(RtcTransportMetricsSnapshot {
        video_rtt_ms,
        video_rtt_source,
        inbound_video_loss_ratio_5s,
        inbound_video_loss_ratio_1s,
        transport_path,
        transport_candidate_pair,
        transport_protocol,
        transport_address_family,
        inbound_video_bitrate_kbps,
        inbound_primary_video_bytes_total: inbound_video_bytes_total,
    })
}

pub(crate) fn publish_transport_metrics_sample(
    runtime_stats: &RuntimeStatsSink,
    snapshot: &RtcTransportMetricsSnapshot,
) {
    runtime_stats.record_transport_metrics(
        snapshot.video_rtt_ms,
        snapshot.video_rtt_source.clone(),
        snapshot.inbound_video_loss_ratio_5s,
        snapshot.inbound_video_loss_ratio_1s,
        snapshot.transport_path.clone(),
        snapshot.transport_candidate_pair.clone(),
        snapshot.transport_protocol.clone(),
        snapshot.transport_address_family.clone(),
        snapshot.inbound_video_bitrate_kbps,
        snapshot.inbound_primary_video_bytes_total,
    );
}

pub(crate) fn build_twcc_observation(
    observation_id: u64,
    packet: &TransportLayerCc,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    source: &'static str,
) -> Option<XbxEngineVideoTwccObservation> {
    build_twcc_observation_with_packet_bytes(observation_id, packet, runtime_stats, source, None)
}

pub(crate) fn build_twcc_observation_with_packet_bytes(
    observation_id: u64,
    packet: &TransportLayerCc,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    source: &'static str,
    packet_bytes_by_transport_seq: Option<&HashMap<u16, u32>>,
) -> Option<XbxEngineVideoTwccObservation> {
    let observed_at_ms = now_ms_f64();
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let observed_packet_count = packet
            .recv_deltas
            .len()
            .min(packet.packet_status_count as usize)
            .min(u16::MAX as usize) as u16;
        if packet.packet_status_count == 0 {
            return None;
        }

        let covered_sequence_span = packet.packet_status_count;
        let covered_sequence_end = packet
            .base_sequence_number
            .wrapping_add(covered_sequence_span.saturating_sub(1));
        let feedback_interval_ms = stats
            .latest_video_twcc_observation
            .as_ref()
            .filter(|previous| previous.source == source)
            .map(|previous| (observed_at_ms - previous.observed_at_ms).max(0.0))
            .filter(|value| *value > 0.0);
        let arrival_span_ms = {
            let span_ms = packet
                .recv_deltas
                .iter()
                .map(|delta| delta.delta.max(0) as f64 / 1_000.0)
                .sum::<f64>();
            (span_ms > 0.0).then_some(span_ms).or(feedback_interval_ms)
        };
        let covered_sequence_span_nonzero = covered_sequence_span.max(1);
        let coverage_ratio = Some(
            (observed_packet_count as f64 / covered_sequence_span_nonzero as f64).clamp(0.0, 1.0),
        );
        let observed_from_ledger = packet_bytes_by_transport_seq
            .map(|ledger| sum_twcc_observed_packet_bytes(packet, ledger))
            .unwrap_or_default();
        let observed_byte_count = observed_from_ledger.observed_byte_count;
        let ledger_hit_ratio = (source == TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK
            && packet_bytes_by_transport_seq.is_some()
            && observed_packet_count > 0)
            .then_some(
                (f64::from(observed_from_ledger.ledger_hit_count) / observed_packet_count as f64)
                    .clamp(0.0, 1.0),
            );
        let sample_gate = evaluate_twcc_sample_gate(
            source,
            observed_packet_count,
            coverage_ratio,
            ledger_hit_ratio,
            observed_byte_count,
            feedback_interval_ms,
            arrival_span_ms,
        );
        let receive_bitrate_kbps = feedback_interval_ms
            .filter(|interval| *interval > 0.0 && observed_byte_count > 0 && sample_gate.is_valid)
            .map(|interval| observed_byte_count as f64 * 8.0 / interval);
        let packet_status_count = packet.packet_status_count.max(1);
        let quality = classify_twcc_observation_quality(
            source,
            packet_status_count,
            observed_packet_count,
            feedback_interval_ms,
        );
        let effective_packet_status_count =
            effective_twcc_packet_status_count(packet_status_count, observed_packet_count, quality);
        let delivery_ratio =
            (observed_packet_count as f64 / effective_packet_status_count as f64).clamp(0.0, 1.0);
        let packet_loss_ratio = (1.0 - delivery_ratio).clamp(0.0, 1.0);

        Some(XbxEngineVideoTwccObservation {
            observation_id,
            source: source.to_string(),
            feedback_packet_count: u16::from(packet.fb_pkt_count),
            covered_sequence_start: packet.base_sequence_number,
            covered_sequence_end,
            covered_sequence_span,
            observed_packet_count,
            observed_byte_count,
            coverage_ratio,
            ledger_hit_ratio,
            feedback_interval_ms,
            arrival_span_ms,
            receive_bitrate_kbps,
            twcc_sample_valid: sample_gate.is_valid,
            twcc_invalid_reason: sample_gate.invalid_reason,
            quality,
            delivery_ratio,
            packet_loss_ratio,
            observed_at_ms,
        })
    })
    .flatten()
}

#[derive(Default)]
struct TwccSampleGateResult {
    is_valid: bool,
    invalid_reason: Option<String>,
}

#[derive(Default)]
struct TwccObservedBytesFromLedger {
    observed_byte_count: u64,
    ledger_hit_count: u16,
}

impl super::RtcConnectionService {
    pub(crate) fn refresh_transport_metrics(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let now_ms = now_ms_f64();
        let previous_sample_at_ms = self.last_transport_metrics_sample_at_ms;
        if now_ms - previous_sample_at_ms < 1_000.0 {
            return;
        }
        self.last_transport_metrics_sample_at_ms = now_ms;

        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return;
        };

        let connected_at_ms =
            matches!(self.lifecycle_state, RtcConnectionLifecycleState::Connected)
                .then_some(self.lifecycle_state_since_ms);
        let previous_inbound_video_bytes_total =
            self.last_transport_metrics_sample_inbound_video_bytes_total;
        let previous_sample_at_ms = (previous_sample_at_ms > 0.0).then_some(previous_sample_at_ms);
        let previous_inbound_video_bytes_total =
            (previous_inbound_video_bytes_total > 0).then_some(previous_inbound_video_bytes_total);
        let Some(snapshot) = collect_transport_metrics(
            peer_connection,
            connected_at_ms,
            previous_sample_at_ms,
            previous_inbound_video_bytes_total,
        ) else {
            return;
        };
        self.last_transport_metrics_sample_inbound_video_bytes_total =
            snapshot.inbound_primary_video_bytes_total;
        let runtime_stats_sink = RuntimeStatsSink::new(runtime_stats.clone());
        publish_transport_metrics_sample(&runtime_stats_sink, &snapshot);
        self.push_transport_fact(TransportFact::Peer(PeerFact::TransportMetricsSampled {
            video_rtt_ms: snapshot.video_rtt_ms,
            loss_ratio_1s: snapshot.inbound_video_loss_ratio_1s,
            actual_video_bitrate_kbps: Some(snapshot.inbound_video_bitrate_kbps),
            observed_remb_kbps: runtime_stats
                .lock()
                .ok()
                .and_then(|stats| stats.video_remb_bps.map(|bps| bps / 1_000)),
            transport_path: snapshot.transport_path.clone(),
            observed_at_ms: now_ms,
        }));
    }
}

fn selected_candidate_pair(
    report: &RTCStatsReport,
) -> Option<&rtc::statistics::stats::ice_candidate_pair::RTCIceCandidatePairStats> {
    let selected_pair_id = report.transport().and_then(|transport| {
        (!transport.selected_candidate_pair_id.is_empty())
            .then_some(transport.selected_candidate_pair_id.as_str())
    });

    if let Some(selected_pair_id) = selected_pair_id {
        if let Some(RTCStatsReportEntry::IceCandidatePair(pair)) = report.get(selected_pair_id) {
            return Some(pair);
        }
    }

    report
        .candidate_pairs()
        .find(|pair| pair.nominated)
        .or_else(|| {
            report
                .candidate_pairs()
                .find(|pair| pair.current_round_trip_time > 0.0)
        })
}

fn select_video_rtt(
    report: &RTCStatsReport,
    selected_pair: &rtc::statistics::stats::ice_candidate_pair::RTCIceCandidatePairStats,
) -> (Option<f64>, Option<String>) {
    if selected_pair.current_round_trip_time > 0.0 {
        return (
            Some(selected_pair.current_round_trip_time * 1_000.0),
            Some("candidate-pair".to_string()),
        );
    }

    let remote_inbound_rtt_ms = report.iter().find_map(|entry| match entry {
        RTCStatsReportEntry::RemoteInboundRtp(stats) if stats.round_trip_time > 0.0 => {
            Some(stats.round_trip_time * 1_000.0)
        }
        _ => None,
    });
    remote_inbound_rtt_ms.map_or((None, None), |value| {
        (Some(value), Some("remote-inbound-rtp".to_string()))
    })
}

fn select_video_inbound_metrics(report: &RTCStatsReport) -> (f64, f64, u64) {
    let selected_video = report
        .inbound_rtp_streams()
        .filter(is_video_inbound_stream)
        .max_by_key(|stream| stream.received_rtp_stream_stats.packets_received);
    let Some(stream) = selected_video else {
        return (0.0, 0.0, 0);
    };

    let packets_lost = stream.received_rtp_stream_stats.packets_lost.max(0) as u64;
    let packets_received = stream.received_rtp_stream_stats.packets_received;
    let total_packets = packets_received.saturating_add(packets_lost);
    let loss_ratio = if total_packets > 0 {
        packets_lost as f64 / total_packets as f64
    } else {
        0.0
    };

    (loss_ratio, loss_ratio, stream.bytes_received)
}

fn estimate_recent_inbound_bitrate_kbps(
    inbound_video_bytes_total: u64,
    previous_inbound_video_bytes_total: Option<u64>,
    connected_at_ms: Option<f64>,
    previous_sample_at_ms: Option<f64>,
    now_ms: f64,
) -> f64 {
    let Some(reference_start_ms) = previous_sample_at_ms.or(connected_at_ms) else {
        return 0.0;
    };
    let elapsed_ms = (now_ms - reference_start_ms).max(0.0);
    if elapsed_ms <= 0.0 {
        return 0.0;
    }
    // 这里按“上一采样点 -> 当前采样点”的窗口估算实时吞吐；
    // 若还没有上一采样点，就退回到连接起点，避免首个样本直接变成 0。
    let baseline_bytes_total = previous_inbound_video_bytes_total.unwrap_or(0);
    let delta_bytes_total = inbound_video_bytes_total.saturating_sub(baseline_bytes_total);
    (delta_bytes_total as f64 * 8.0 / elapsed_ms).max(0.0)
}

fn sum_twcc_observed_packet_bytes(
    packet: &TransportLayerCc,
    packet_bytes_by_transport_seq: &HashMap<u16, u32>,
) -> TwccObservedBytesFromLedger {
    let mut observed_byte_count = 0u64;
    let mut ledger_hit_count = 0u16;
    twcc_received_sequences(packet)
        .into_iter()
        .filter_map(|sequence| packet_bytes_by_transport_seq.get(&sequence).copied())
        .for_each(|packet_bytes| {
            observed_byte_count = observed_byte_count.saturating_add(u64::from(packet_bytes));
            ledger_hit_count = ledger_hit_count.saturating_add(1);
        });
    TwccObservedBytesFromLedger {
        observed_byte_count,
        ledger_hit_count,
    }
}

fn twcc_received_sequences(packet: &TransportLayerCc) -> Vec<u16> {
    let mut remaining = packet.packet_status_count as usize;
    let mut sequence = packet.base_sequence_number;
    let mut received_sequences = Vec::new();
    for chunk in &packet.packet_chunks {
        if remaining == 0 {
            break;
        }
        match chunk {
            rtc_rtcp::transport_feedbacks::transport_layer_cc::PacketStatusChunk::RunLengthChunk(
                chunk,
            ) => {
                let run_length = usize::from(chunk.run_length).min(remaining);
                for _ in 0..run_length {
                    if twcc_symbol_is_received(chunk.packet_status_symbol) {
                        received_sequences.push(sequence);
                    }
                    sequence = sequence.wrapping_add(1);
                }
                remaining = remaining.saturating_sub(run_length);
            }
            rtc_rtcp::transport_feedbacks::transport_layer_cc::PacketStatusChunk::StatusVectorChunk(
                chunk,
            ) => {
                for symbol in &chunk.symbol_list {
                    if remaining == 0 {
                        break;
                    }
                    if twcc_symbol_is_received(*symbol) {
                        received_sequences.push(sequence);
                    }
                    sequence = sequence.wrapping_add(1);
                    remaining = remaining.saturating_sub(1);
                }
            }
        }
    }
    received_sequences
}

fn twcc_symbol_is_received(
    symbol: rtc_rtcp::transport_feedbacks::transport_layer_cc::SymbolTypeTcc,
) -> bool {
    !matches!(
        symbol,
        rtc_rtcp::transport_feedbacks::transport_layer_cc::SymbolTypeTcc::PacketNotReceived
    )
}

fn evaluate_twcc_sample_gate(
    source: &'static str,
    observed_packet_count: u16,
    coverage_ratio: Option<f64>,
    ledger_hit_ratio: Option<f64>,
    observed_byte_count: u64,
    feedback_interval_ms: Option<f64>,
    arrival_span_ms: Option<f64>,
) -> TwccSampleGateResult {
    if source != TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK {
        return TwccSampleGateResult {
            is_valid: true,
            invalid_reason: None,
        };
    }

    const MIN_VALID_OBSERVED_PACKETS: u16 = 8;
    const MAX_VALID_SAMPLE_INTERVAL_MS: f64 = 500.0;
    const MIN_COVERAGE_RATIO: f64 = 0.60;
    const MIN_LEDGER_HIT_RATIO: f64 = 0.90;

    let mut reasons = Vec::<String>::new();
    if observed_packet_count == 0 {
        reasons.push("no-observed-packets".to_string());
    }
    if observed_packet_count > 0 && observed_byte_count == 0 {
        reasons.push("missing-byte-ledger".to_string());
    }
    if observed_packet_count < MIN_VALID_OBSERVED_PACKETS {
        reasons.push("sample-too-small".to_string());
    }
    if coverage_ratio.is_some_and(|ratio| ratio < MIN_COVERAGE_RATIO) {
        reasons.push("coverage-too-low".to_string());
    }
    if ledger_hit_ratio.is_some_and(|ratio| ratio < MIN_LEDGER_HIT_RATIO) {
        reasons.push("ledger-hit-too-low".to_string());
    }
    if let Some(sample_interval_ms) = feedback_interval_ms
        .or(arrival_span_ms)
        .filter(|interval_ms| *interval_ms > MAX_VALID_SAMPLE_INTERVAL_MS)
    {
        let mut reason = String::from("interval-too-long:");
        let _ = write!(&mut reason, "{sample_interval_ms:.1}");
        reasons.push(reason);
    }

    let invalid_reason = if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("|"))
    };
    TwccSampleGateResult {
        is_valid: invalid_reason.is_none(),
        invalid_reason,
    }
}

fn classify_twcc_observation_quality(
    source: &'static str,
    packet_status_count: u16,
    observed_packet_count: u16,
    feedback_interval_ms: Option<f64>,
) -> XbxEngineTwccObservationQuality {
    if source != TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK {
        return XbxEngineTwccObservationQuality::Stable;
    }
    if feedback_interval_ms.is_some() || observed_packet_count == 0 {
        return XbxEngineTwccObservationQuality::Stable;
    }
    if packet_status_count > observed_packet_count.saturating_mul(3) {
        return XbxEngineTwccObservationQuality::BootstrapSparse;
    }
    XbxEngineTwccObservationQuality::Unstable
}

fn effective_twcc_packet_status_count(
    packet_status_count: u16,
    observed_packet_count: u16,
    quality: XbxEngineTwccObservationQuality,
) -> u16 {
    if quality == XbxEngineTwccObservationQuality::BootstrapSparse {
        return observed_packet_count.max(1);
    }
    packet_status_count.max(1)
}

fn candidate_type_for(report: &RTCStatsReport, candidate_id: &str) -> Option<RTCIceCandidateType> {
    match report.get(candidate_id)? {
        RTCStatsReportEntry::LocalCandidate(candidate) => Some(candidate.candidate_type),
        RTCStatsReportEntry::RemoteCandidate(candidate) => Some(candidate.candidate_type),
        _ => None,
    }
}

fn candidate_protocol_for(report: &RTCStatsReport, candidate_id: &str) -> Option<String> {
    match report.get(candidate_id)? {
        RTCStatsReportEntry::LocalCandidate(candidate)
        | RTCStatsReportEntry::RemoteCandidate(candidate) => {
            let protocol = candidate.protocol.trim();
            (!protocol.is_empty()).then_some(protocol.to_ascii_uppercase())
        }
        _ => None,
    }
}

fn candidate_address_family_for(
    report: &RTCStatsReport,
    candidate_id: &str,
) -> TransportAddressFamily {
    match report.get(candidate_id) {
        Some(RTCStatsReportEntry::LocalCandidate(candidate))
        | Some(RTCStatsReportEntry::RemoteCandidate(candidate)) => {
            resolve_candidate_address_family(candidate.address.as_deref())
        }
        _ => TransportAddressFamily::Unknown,
    }
}

fn resolve_candidate_address_family(address: Option<&str>) -> TransportAddressFamily {
    let Some(raw_address) = address.map(str::trim).filter(|value| !value.is_empty()) else {
        return TransportAddressFamily::Unknown;
    };
    let without_prefix = raw_address.strip_prefix('[').unwrap_or(raw_address);
    let normalized = without_prefix.strip_suffix(']').unwrap_or(without_prefix);
    if normalized.contains(':') {
        return TransportAddressFamily::Ipv6;
    }
    if normalized.contains('.') {
        return TransportAddressFamily::Ipv4;
    }
    TransportAddressFamily::Unknown
}

fn normalize_candidate_type(candidate_type: RTCIceCandidateType) -> &'static str {
    match candidate_type {
        RTCIceCandidateType::Host => "host",
        RTCIceCandidateType::Srflx => "srflx",
        RTCIceCandidateType::Prflx => "prflx",
        RTCIceCandidateType::Relay => "relay",
        _ => "unknown",
    }
}

fn build_transport_candidate_pair(
    local_candidate_type: Option<RTCIceCandidateType>,
    remote_candidate_type: Option<RTCIceCandidateType>,
) -> Option<String> {
    match (local_candidate_type, remote_candidate_type) {
        (None, None) => None,
        (local, remote) => Some(format!(
            "{}->{}",
            local.map(normalize_candidate_type).unwrap_or("unknown"),
            remote.map(normalize_candidate_type).unwrap_or("unknown"),
        )),
    }
}

fn resolve_transport_protocol(local: Option<String>, remote: Option<String>) -> Option<String> {
    match (local, remote) {
        (Some(local), Some(remote)) if local.eq_ignore_ascii_case(remote.as_str()) => Some(local),
        (Some(local), Some(remote)) => Some(format!("{local}/{remote}")),
        (Some(local), None) => Some(local),
        (None, Some(remote)) => Some(remote),
        (None, None) => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportAddressFamily {
    Ipv4,
    Ipv6,
    Unknown,
}

impl TransportAddressFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for TransportAddressFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn resolve_transport_address_family(
    local: TransportAddressFamily,
    remote: TransportAddressFamily,
) -> &'static str {
    match (local, remote) {
        (TransportAddressFamily::Unknown, TransportAddressFamily::Unknown) => "unknown",
        (TransportAddressFamily::Unknown, TransportAddressFamily::Ipv4)
        | (TransportAddressFamily::Ipv4, TransportAddressFamily::Unknown)
        | (TransportAddressFamily::Ipv4, TransportAddressFamily::Ipv4) => "ipv4",
        (TransportAddressFamily::Unknown, TransportAddressFamily::Ipv6)
        | (TransportAddressFamily::Ipv6, TransportAddressFamily::Unknown)
        | (TransportAddressFamily::Ipv6, TransportAddressFamily::Ipv6) => "ipv6",
        (TransportAddressFamily::Ipv4, TransportAddressFamily::Ipv6)
        | (TransportAddressFamily::Ipv6, TransportAddressFamily::Ipv4) => "mixed",
    }
}

fn candidate_summary(report: &RTCStatsReport, candidate_id: &str) -> String {
    match report.get(candidate_id) {
        Some(RTCStatsReportEntry::LocalCandidate(candidate))
        | Some(RTCStatsReportEntry::RemoteCandidate(candidate)) => format!(
            "type={:?} addr={}:{} related={}:{} protocol={} url={}",
            candidate.candidate_type,
            candidate.address.as_deref().unwrap_or("?"),
            candidate.port,
            if candidate.related_address.is_empty() {
                "?"
            } else {
                candidate.related_address.as_str()
            },
            candidate.related_port,
            candidate.protocol,
            if candidate.url.is_empty() {
                "-"
            } else {
                candidate.url.as_str()
            },
        ),
        _ => format!("id={candidate_id}"),
    }
}

fn classify_transport_path(
    local_candidate_type: Option<RTCIceCandidateType>,
    remote_candidate_type: Option<RTCIceCandidateType>,
) -> Option<String> {
    match (local_candidate_type, remote_candidate_type) {
        (Some(RTCIceCandidateType::Relay), _) | (_, Some(RTCIceCandidateType::Relay)) => {
            Some("Relay".to_string())
        }
        (Some(RTCIceCandidateType::Host), Some(RTCIceCandidateType::Host)) => {
            Some("Direct (host->host)".to_string())
        }
        (Some(RTCIceCandidateType::Host), Some(RTCIceCandidateType::Srflx))
        | (Some(RTCIceCandidateType::Srflx), Some(RTCIceCandidateType::Host)) => {
            Some("Direct (host->srflx)".to_string())
        }
        (Some(RTCIceCandidateType::Srflx), Some(RTCIceCandidateType::Srflx)) => {
            Some("Direct (srflx->srflx)".to_string())
        }
        (Some(_), Some(_)) => Some("Direct".to_string()),
        (Some(_), None) | (None, Some(_)) | (None, None) => Some("Direct".to_string()),
    }
}

fn is_video_inbound_stream(
    stream: &&rtc::statistics::stats::rtp_stream::received::inbound::RTCInboundRtpStreamStats,
) -> bool {
    is_video_inbound_stream_by_hints(
        stream.received_rtp_stream_stats.rtp_stream_stats.kind,
        stream.frame_width,
        stream.frame_height,
        stream.frames_decoded,
        stream.frames_rendered,
        stream.decoder_implementation.as_str(),
    )
}

fn is_video_inbound_stream_by_hints(
    kind: RtpCodecKind,
    frame_width: u32,
    frame_height: u32,
    frames_decoded: u32,
    frames_rendered: u32,
    decoder_implementation: &str,
) -> bool {
    // 优先按 RTP stats 的媒体类型判定视频流；
    // 解码维度字段在某些平台/时序下会长期为 0，不能再作为硬门槛。
    kind == RtpCodecKind::Video
        || frame_width > 0
        || frame_height > 0
        || frames_decoded > 0
        || frames_rendered > 0
        || decoder_implementation.eq_ignore_ascii_case("video")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use rtc_rtcp::transport_feedbacks::transport_layer_cc::{
        PacketStatusChunk, StatusChunkTypeTcc, StatusVectorChunk, SymbolSizeTypeTcc,
    };
    use rtc_rtcp::transport_feedbacks::transport_layer_cc::{SymbolTypeTcc, TransportLayerCc};

    use super::{
        build_transport_candidate_pair, build_twcc_observation,
        build_twcc_observation_with_packet_bytes, classify_transport_path,
        estimate_recent_inbound_bitrate_kbps, is_video_inbound_stream_by_hints,
        resolve_candidate_address_family, resolve_transport_address_family,
        resolve_transport_protocol, TransportAddressFamily, TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        TWCC_OBSERVATION_SOURCE_REMOTE_RTCP,
    };
    use crate::XbxEngineMediaRuntimeStats;
    use rtc::peer_connection::transport::RTCIceCandidateType;
    use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;

    #[test]
    fn relay_path_is_detected_from_either_side() {
        assert_eq!(
            classify_transport_path(
                Some(RTCIceCandidateType::Host),
                Some(RTCIceCandidateType::Relay),
            ),
            Some("Relay".to_string())
        );
        assert_eq!(
            classify_transport_path(
                Some(RTCIceCandidateType::Relay),
                Some(RTCIceCandidateType::Host),
            ),
            Some("Relay".to_string())
        );
    }

    #[test]
    fn direct_path_is_kept_for_non_relay_pairs() {
        assert_eq!(
            classify_transport_path(
                Some(RTCIceCandidateType::Host),
                Some(RTCIceCandidateType::Srflx),
            ),
            Some("Direct (host->srflx)".to_string())
        );
    }

    #[test]
    fn direct_path_falls_back_when_candidate_types_are_missing() {
        assert_eq!(
            classify_transport_path(None, None),
            Some("Direct".to_string())
        );
    }

    #[test]
    fn transport_candidate_pair_is_normalized_to_lowercase() {
        assert_eq!(
            build_transport_candidate_pair(
                Some(RTCIceCandidateType::Host),
                Some(RTCIceCandidateType::Srflx)
            ),
            Some("host->srflx".to_string())
        );
    }

    #[test]
    fn transport_protocol_prefers_single_value_when_consistent() {
        assert_eq!(
            resolve_transport_protocol(Some("UDP".to_string()), Some("udp".to_string())),
            Some("UDP".to_string())
        );
    }

    #[test]
    fn transport_protocol_marks_mixed_sources() {
        assert_eq!(
            resolve_transport_protocol(Some("UDP".to_string()), Some("TCP".to_string())),
            Some("UDP/TCP".to_string())
        );
    }

    #[test]
    fn transport_address_family_uses_mixed_when_local_and_remote_differ() {
        assert_eq!(
            resolve_transport_address_family(
                TransportAddressFamily::Ipv4,
                TransportAddressFamily::Ipv6
            ),
            "mixed"
        );
    }

    #[test]
    fn transport_address_family_falls_back_to_known_side() {
        assert_eq!(
            resolve_transport_address_family(
                TransportAddressFamily::Unknown,
                TransportAddressFamily::Ipv6
            ),
            "ipv6"
        );
    }

    #[test]
    fn candidate_address_family_detects_ipv4_ipv6_and_unknown() {
        assert_eq!(
            resolve_candidate_address_family(Some("192.168.0.2")),
            TransportAddressFamily::Ipv4
        );
        assert_eq!(
            resolve_candidate_address_family(Some("[2001:db8::1]")),
            TransportAddressFamily::Ipv6
        );
        assert_eq!(
            resolve_candidate_address_family(Some("")),
            TransportAddressFamily::Unknown
        );
    }

    #[test]
    fn recent_inbound_bitrate_uses_window_delta_when_previous_sample_exists() {
        let bitrate = estimate_recent_inbound_bitrate_kbps(
            1_800_000,
            Some(900_000),
            Some(500.0),
            Some(1_000.0),
            2_000.0,
        );
        assert_eq!(bitrate, 7_200.0);
    }

    #[test]
    fn recent_inbound_bitrate_falls_back_to_connection_start_on_first_sample() {
        let bitrate =
            estimate_recent_inbound_bitrate_kbps(900_000, None, Some(500.0), None, 1_500.0);
        assert_eq!(bitrate, 7_200.0);
    }

    #[test]
    fn video_kind_is_accepted_even_when_frame_decode_hints_are_zero() {
        assert!(is_video_inbound_stream_by_hints(
            RtpCodecKind::Video,
            0,
            0,
            0,
            0,
            "",
        ));
    }

    #[test]
    fn audio_kind_without_video_hints_is_rejected() {
        assert!(!is_video_inbound_stream_by_hints(
            RtpCodecKind::Audio,
            0,
            0,
            0,
            0,
            "",
        ));
    }

    #[test]
    fn audio_kind_with_video_decode_hints_still_uses_fallback_video_detection() {
        assert!(is_video_inbound_stream_by_hints(
            RtpCodecKind::Audio,
            1920,
            1080,
            0,
            0,
            "",
        ));
    }

    #[test]
    fn remote_rtcp_twcc_observation_does_not_fallback_to_video_transport_bitrate() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats {
            inbound_video_bitrate_kbps: Some(9_000.0),
            ..XbxEngineMediaRuntimeStats::default()
        }));
        let packet = TransportLayerCc {
            packet_status_count: 10,
            ..TransportLayerCc::default()
        };

        let observation = build_twcc_observation(
            1,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_REMOTE_RTCP,
        )
        .expect("twcc observation should be built");

        assert_eq!(observation.observed_byte_count, 0);
        assert_eq!(observation.receive_bitrate_kbps, None);
    }

    #[test]
    fn local_feedback_twcc_observation_without_ledger_is_marked_invalid() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats {
            inbound_video_bitrate_kbps: Some(9_000.0),
            ..XbxEngineMediaRuntimeStats::default()
        }));
        let packet = TransportLayerCc {
            packet_status_count: 10,
            ..TransportLayerCc::default()
        };

        let observation = build_twcc_observation(
            1,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        )
        .expect("twcc observation should be built");

        assert_eq!(observation.receive_bitrate_kbps, None);
        assert_eq!(observation.coverage_ratio, Some(0.0));
        assert_eq!(observation.ledger_hit_ratio, None);
        assert!(!observation.twcc_sample_valid);
        assert!(observation
            .twcc_invalid_reason
            .as_deref()
            .unwrap_or("")
            .contains("sample-too-small"));
    }

    #[test]
    fn first_local_feedback_twcc_uses_optimistic_delivery_ratio_when_interval_is_missing() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let packet = TransportLayerCc {
            packet_status_count: 93,
            recv_deltas: std::iter::repeat_n(
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 1_000,
                },
                17,
            )
            .collect(),
            ..TransportLayerCc::default()
        };

        let observation = build_twcc_observation(
            1,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        )
        .expect("twcc observation should be built");

        assert_eq!(observation.feedback_interval_ms, None);
        assert_eq!(observation.observed_packet_count, 17);
        assert_eq!(observation.covered_sequence_span, 93);
        assert!(observation.coverage_ratio.is_some());
        assert!(observation.ledger_hit_ratio.is_none());
        assert_eq!(observation.delivery_ratio, 1.0);
        assert_eq!(observation.packet_loss_ratio, 0.0);
    }

    #[test]
    fn local_feedback_twcc_observation_uses_packet_byte_ledger_when_available() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let packet = TransportLayerCc {
            base_sequence_number: 100,
            packet_status_count: 8,
            packet_chunks: vec![PacketStatusChunk::StatusVectorChunk(StatusVectorChunk {
                type_tcc: StatusChunkTypeTcc::StatusVectorChunk,
                symbol_size: SymbolSizeTypeTcc::TwoBit,
                symbol_list: vec![
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                    SymbolTypeTcc::PacketReceivedSmallDelta,
                ],
            })],
            recv_deltas: vec![
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
            ],
            ..TransportLayerCc::default()
        };
        let mut ledger = HashMap::new();
        ledger.insert(100, 1200);
        ledger.insert(101, 1300);
        ledger.insert(102, 1400);
        ledger.insert(103, 1500);
        ledger.insert(104, 1600);
        ledger.insert(105, 1700);
        ledger.insert(106, 1800);
        ledger.insert(107, 1900);

        let mut first_observation = build_twcc_observation_with_packet_bytes(
            1,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
            Some(&ledger),
        )
        .expect("twcc observation should be built");
        assert!(first_observation.twcc_sample_valid);
        assert_eq!(first_observation.observed_byte_count, 12400);
        assert_eq!(first_observation.coverage_ratio, Some(1.0));
        assert_eq!(first_observation.ledger_hit_ratio, Some(1.0));
        assert_eq!(first_observation.receive_bitrate_kbps, None);
        // 避免两次观测落在同一毫秒，导致反馈间隔偶发为 None。
        first_observation.observed_at_ms = (first_observation.observed_at_ms - 1.0).max(0.0);

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.latest_video_twcc_observation = Some(first_observation);
        }

        let second_observation = build_twcc_observation_with_packet_bytes(
            2,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
            Some(&ledger),
        )
        .expect("twcc observation should be built");
        assert!(second_observation.twcc_sample_valid);
        assert_eq!(second_observation.observed_byte_count, 12400);
        assert_eq!(second_observation.coverage_ratio, Some(1.0));
        assert_eq!(second_observation.ledger_hit_ratio, Some(1.0));
        assert!(second_observation.feedback_interval_ms.is_some());
        assert!(second_observation.receive_bitrate_kbps.is_some());
    }

    #[test]
    fn local_feedback_twcc_observation_marks_invalid_when_sample_interval_too_long() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let packet = TransportLayerCc {
            packet_status_count: 8,
            recv_deltas: std::iter::repeat_n(
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 80_000,
                },
                8,
            )
            .collect(),
            ..TransportLayerCc::default()
        };

        let observation = build_twcc_observation(
            1,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        )
        .expect("twcc observation should be built");
        assert!(!observation.twcc_sample_valid);
        assert!(observation
            .twcc_invalid_reason
            .as_deref()
            .unwrap_or("")
            .contains("interval-too-long"));
    }

    #[test]
    fn local_feedback_twcc_observation_marks_invalid_when_coverage_too_low() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let packet = TransportLayerCc {
            base_sequence_number: 100,
            packet_status_count: 10,
            packet_chunks: vec![PacketStatusChunk::RunLengthChunk(
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RunLengthChunk {
                    type_tcc: StatusChunkTypeTcc::RunLengthChunk,
                    packet_status_symbol: SymbolTypeTcc::PacketReceivedSmallDelta,
                    run_length: 4,
                },
            )],
            recv_deltas: std::iter::repeat_n(
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                4,
            )
            .collect(),
            ..TransportLayerCc::default()
        };
        let mut ledger = HashMap::new();
        ledger.insert(100, 1200);
        ledger.insert(101, 1200);
        ledger.insert(102, 1200);
        ledger.insert(103, 1200);

        let observation = build_twcc_observation_with_packet_bytes(
            1,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
            Some(&ledger),
        )
        .expect("twcc observation should be built");

        assert_eq!(observation.coverage_ratio, Some(0.4));
        assert!(!observation.twcc_sample_valid);
        assert!(observation
            .twcc_invalid_reason
            .as_deref()
            .unwrap_or("")
            .contains("coverage-too-low"));
    }

    #[test]
    fn local_feedback_twcc_observation_marks_invalid_when_ledger_hit_too_low() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let packet = TransportLayerCc {
            base_sequence_number: 100,
            packet_status_count: 10,
            packet_chunks: vec![PacketStatusChunk::RunLengthChunk(
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RunLengthChunk {
                    type_tcc: StatusChunkTypeTcc::RunLengthChunk,
                    packet_status_symbol: SymbolTypeTcc::PacketReceivedSmallDelta,
                    run_length: 10,
                },
            )],
            recv_deltas: std::iter::repeat_n(
                rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                    type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 10_000,
                },
                10,
            )
            .collect(),
            ..TransportLayerCc::default()
        };
        let mut ledger = HashMap::new();
        ledger.insert(100, 1200);
        ledger.insert(101, 1200);
        ledger.insert(102, 1200);
        ledger.insert(103, 1200);
        ledger.insert(104, 1200);

        let observation = build_twcc_observation_with_packet_bytes(
            1,
            &packet,
            &runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
            Some(&ledger),
        )
        .expect("twcc observation should be built");

        assert_eq!(observation.coverage_ratio, Some(1.0));
        assert_eq!(observation.ledger_hit_ratio, Some(0.5));
        assert!(!observation.twcc_sample_valid);
        assert!(observation
            .twcc_invalid_reason
            .as_deref()
            .unwrap_or("")
            .contains("ledger-hit-too-low"));
    }
}
