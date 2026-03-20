use std::sync::Arc;

use webrtc::{peer_connection::RTCPeerConnection, track::track_remote::TrackRemote};

use crate::{
    runtime_stats_sink::RuntimeStatsSink,
    transport::webrtc::transport_observation::{
        candidate_pair_average_rtt, resolve_transport_path, select_any_candidate_pair_rtt,
        select_preferred_candidate_pair,
    },
};

use super::now_ms_f64;

// 仅维护 transport 观测采样需要的增量状态，不承载 BWE/policy 决策字段。
pub(super) struct VideoTrackObservationCollectorState {
    last_bytes_received: u64,
    last_packets_received: u64,
    last_video_sample_at_ms: f64,
    last_loss_estimate_total: u64,
    last_loss_recovered_total: u64,
    last_loss_finalized_total: u64,
}

pub(super) struct VideoTrackTransportObservation {
    pub current_bytes: u64,
    pub packets_received: u64,
    pub actual_kbps: f64,
    pub fraction_lost: f64,
    pub synthetic_loss_ratio: f64,
    pub observed_remb_kbps: Option<u32>,
    pub rtt_ms: Option<f64>,
    pub rtt_source: Option<String>,
    pub transport_path: Option<String>,
    pub sampled_at_ms: f64,
    pub should_mark_video_started: bool,
}

impl VideoTrackObservationCollectorState {
    pub(super) fn new(sampled_at_ms: f64) -> Self {
        Self {
            last_bytes_received: 0,
            last_packets_received: 0,
            last_video_sample_at_ms: sampled_at_ms,
            last_loss_estimate_total: 0,
            last_loss_recovered_total: 0,
            last_loss_finalized_total: 0,
        }
    }

    pub(super) async fn collect(
        &mut self,
        stats_track: &Arc<TrackRemote>,
        peer_connection: &Arc<RTCPeerConnection>,
        runtime_stats: &RuntimeStatsSink,
    ) -> Option<VideoTrackTransportObservation> {
        let stats = peer_connection.get_stats().await;
        let mut current_bytes = 0;
        let mut packets_received = 0u64;
        let mut rtt_seconds = 0.0f64;
        let mut rtt_source: Option<&'static str> = None;
        let mut fraction_lost = 0.0f64;
        let mut candidate_pair_rtt = 0.0f64;
        let mut candidate_pair_avg_rtt = 0.0f64;
        let mut synthetic_loss_ratio = 0.0f64;
        let mut avail_bps = 0.0f64;
        let transport_path = resolve_transport_path(&stats);
        let selected_candidate_pair = select_preferred_candidate_pair(&stats);

        for report in stats.reports.values() {
            match report {
                webrtc::stats::StatsReportType::InboundRTP(inbound) => {
                    if inbound.ssrc == stats_track.ssrc() {
                        current_bytes = inbound.bytes_received;
                        packets_received = inbound.packets_received;
                    }
                }
                webrtc::stats::StatsReportType::CandidatePair(pair) => {
                    if pair.available_outgoing_bitrate > avail_bps {
                        avail_bps = pair.available_outgoing_bitrate;
                    }
                    if let Some(selected_pair) = selected_candidate_pair {
                        if pair.id == selected_pair.id {
                            if pair.current_round_trip_time > 0.0 {
                                candidate_pair_rtt = pair.current_round_trip_time;
                            }
                            let avg_rtt = candidate_pair_average_rtt(pair);
                            if avg_rtt > 0.0 {
                                candidate_pair_avg_rtt = avg_rtt;
                            }
                        }
                    }
                }
                webrtc::stats::StatsReportType::RemoteInboundRTP(remote_inbound) => {
                    if remote_inbound.ssrc == stats_track.ssrc() {
                        fraction_lost = remote_inbound.fraction_lost;
                        if let Some(remote_inbound_rtt) = remote_inbound.round_trip_time {
                            if remote_inbound_rtt > 0.0 {
                                rtt_seconds = remote_inbound_rtt;
                                rtt_source = Some("remote-inbound");
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if rtt_seconds <= 0.0 && candidate_pair_rtt > 0.0 {
            rtt_seconds = candidate_pair_rtt;
            rtt_source = Some("candidate-pair");
        } else if rtt_seconds <= 0.0 && candidate_pair_avg_rtt > 0.0 {
            rtt_seconds = candidate_pair_avg_rtt;
            rtt_source = Some("candidate-pair-avg");
        } else if rtt_seconds <= 0.0 {
            if let Some((fallback_rtt, fallback_source)) = select_any_candidate_pair_rtt(&stats) {
                rtt_seconds = fallback_rtt;
                rtt_source = Some(fallback_source);
            }
        }

        let sample_now_ms = now_ms_f64();
        let elapsed_ms = (sample_now_ms - self.last_video_sample_at_ms).max(0.0);
        let delta_bytes = current_bytes.saturating_sub(self.last_bytes_received);
        let should_mark_video_started = current_bytes > 0 && self.last_bytes_received == 0;
        self.last_bytes_received = current_bytes;
        let delta_packets_received = packets_received.saturating_sub(self.last_packets_received);
        self.last_packets_received = packets_received;
        let actual_kbps = (delta_bytes * 8) as f64 / elapsed_ms.max(1.0);
        self.last_video_sample_at_ms = sample_now_ms;

        let Some((
            delta_loss_estimate,
            delta_loss_recovered,
            delta_loss_finalized,
            video_nack_recovery_rtt_ms,
        )) = runtime_stats.read(|shared| {
            let delta_loss_estimate = shared
                .inbound_video_packet_loss_estimate_total
                .saturating_sub(self.last_loss_estimate_total);
            let delta_loss_recovered = shared
                .video_loss_recovered_count_total
                .saturating_sub(self.last_loss_recovered_total);
            let delta_loss_finalized = shared
                .video_loss_finalized_count_total
                .saturating_sub(self.last_loss_finalized_total);
            self.last_loss_estimate_total = shared.inbound_video_packet_loss_estimate_total;
            self.last_loss_recovered_total = shared.video_loss_recovered_count_total;
            self.last_loss_finalized_total = shared.video_loss_finalized_count_total;
            (
                delta_loss_estimate,
                delta_loss_recovered,
                delta_loss_finalized,
                shared.video_nack_recovery_rtt_ms,
            )
        })
        else {
            return None;
        };

        let effective_loss_packets =
            delta_loss_finalized.max(delta_loss_estimate.saturating_sub(delta_loss_recovered));
        let loss_denominator = delta_packets_received.saturating_add(effective_loss_packets);
        if loss_denominator > 0 {
            synthetic_loss_ratio = effective_loss_packets as f64 / loss_denominator as f64;
        }
        if fraction_lost <= 0.0 && synthetic_loss_ratio > 0.0 {
            fraction_lost = synthetic_loss_ratio;
        }
        if rtt_seconds <= 0.0 {
            if let Some(nack_rtt_ms) = video_nack_recovery_rtt_ms {
                rtt_seconds = nack_rtt_ms / 1000.0;
                rtt_source = Some("nack-recovery");
            }
        }

        let observed_remb_kbps = if avail_bps > 0.0 {
            Some((avail_bps / 1000.0).round().max(0.0) as u32)
        } else {
            None
        };

        Some(VideoTrackTransportObservation {
            current_bytes,
            packets_received,
            actual_kbps: actual_kbps.max(0.0),
            fraction_lost,
            synthetic_loss_ratio,
            observed_remb_kbps,
            rtt_ms: (rtt_seconds > 0.0).then_some(rtt_seconds * 1000.0),
            rtt_source: rtt_source.map(str::to_string),
            transport_path,
            sampled_at_ms: sample_now_ms,
            should_mark_video_started,
        })
    }
}
