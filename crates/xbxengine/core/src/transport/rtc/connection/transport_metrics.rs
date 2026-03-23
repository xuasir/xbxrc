use std::time::Instant;

use rtc::peer_connection::transport::RTCIceCandidateType;
use rtc::peer_connection::RTCPeerConnection;
use rtc::statistics::report::{RTCStatsReport, RTCStatsReportEntry};
use rtc::statistics::StatsSelector;

use crate::transport::rtc::stats::now_ms_f64;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RtcTransportMetricsSnapshot {
    pub(crate) video_rtt_ms: Option<f64>,
    pub(crate) video_rtt_source: Option<String>,
    pub(crate) inbound_video_loss_ratio_5s: f64,
    pub(crate) inbound_video_loss_ratio_1s: f64,
    pub(crate) transport_path: Option<String>,
    pub(crate) inbound_video_bitrate_kbps: f64,
    pub(crate) inbound_primary_video_bytes_total: u64,
}

pub(crate) fn collect_transport_metrics(
    peer_connection: &mut RTCPeerConnection,
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

fn collect_transport_metrics_from_report(
    report: &RTCStatsReport,
    connected_at_ms: Option<f64>,
    previous_sample_at_ms: Option<f64>,
    previous_inbound_video_bytes_total: Option<u64>,
) -> Option<RtcTransportMetricsSnapshot> {
    let now_ms = now_ms_f64();
    let selected_pair = selected_candidate_pair(report)?;
    let (video_rtt_ms, video_rtt_source) = select_video_rtt(report, selected_pair);
    let transport_path = classify_transport_path(
        candidate_type_for(report, &selected_pair.local_candidate_id),
        candidate_type_for(report, &selected_pair.remote_candidate_id),
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
        inbound_video_bitrate_kbps,
        inbound_primary_video_bytes_total: inbound_video_bytes_total,
    })
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

fn candidate_type_for(report: &RTCStatsReport, candidate_id: &str) -> Option<RTCIceCandidateType> {
    match report.get(candidate_id)? {
        RTCStatsReportEntry::LocalCandidate(candidate) => Some(candidate.candidate_type),
        RTCStatsReportEntry::RemoteCandidate(candidate) => Some(candidate.candidate_type),
        _ => None,
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
    stream.frame_width > 0
        || stream.frame_height > 0
        || stream.frames_decoded > 0
        || stream.frames_rendered > 0
        || stream.decoder_implementation.eq_ignore_ascii_case("video")
}

#[cfg(test)]
mod tests {
    use super::{classify_transport_path, estimate_recent_inbound_bitrate_kbps};
    use rtc::peer_connection::transport::RTCIceCandidateType;

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
}
