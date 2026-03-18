use webrtc::stats::{ICECandidateStats, StatsReport};

pub(crate) fn resolve_transport_path(stats: &StatsReport) -> Option<String> {
    let selected_pair = select_preferred_candidate_pair(stats);
    let mut local_candidates = std::collections::HashMap::<&str, &ICECandidateStats>::new();
    let mut remote_candidates = std::collections::HashMap::<&str, &ICECandidateStats>::new();

    for report in stats.reports.values() {
        match report {
            webrtc::stats::StatsReportType::LocalCandidate(candidate) => {
                local_candidates.insert(candidate.id.as_str(), candidate);
            }
            webrtc::stats::StatsReportType::RemoteCandidate(candidate) => {
                remote_candidates.insert(candidate.id.as_str(), candidate);
            }
            _ => {}
        }
    }

    let pair = selected_pair?;
    let local_candidate = local_candidates.get(pair.local_candidate_id.as_str())?;
    let remote_candidate = remote_candidates.get(pair.remote_candidate_id.as_str())?;
    let local_type = normalize_candidate_type(&local_candidate.candidate_type);
    let remote_type = normalize_candidate_type(&remote_candidate.candidate_type);
    let path_kind = if local_type == "relay" || remote_type == "relay" {
        "Relay"
    } else {
        "Direct"
    };
    Some(format!("{path_kind} ({local_type}->{remote_type})"))
}

pub(crate) fn select_preferred_candidate_pair(
    stats: &StatsReport,
) -> Option<&webrtc::stats::ICECandidatePairStats> {
    let mut nominated_pair: Option<&webrtc::stats::ICECandidatePairStats> = None;
    let mut active_pair: Option<&webrtc::stats::ICECandidatePairStats> = None;

    for report in stats.reports.values() {
        let webrtc::stats::StatsReportType::CandidatePair(pair) = report else {
            continue;
        };
        if pair.nominated {
            nominated_pair = Some(pair);
            break;
        }
        if active_pair.is_none()
            && (pair.available_outgoing_bitrate > 0.0
                || pair.available_incoming_bitrate > 0.0
                || pair.current_round_trip_time > 0.0)
        {
            active_pair = Some(pair);
        }
    }

    nominated_pair.or(active_pair)
}

pub(crate) fn candidate_pair_average_rtt(pair: &webrtc::stats::ICECandidatePairStats) -> f64 {
    let response_count = pair.responses_received.max(pair.responses_sent) as f64;
    if response_count <= 0.0 || pair.total_round_trip_time <= 0.0 {
        return 0.0;
    }

    pair.total_round_trip_time / response_count
}

pub(crate) fn select_any_candidate_pair_rtt(stats: &StatsReport) -> Option<(f64, &'static str)> {
    let mut candidate_pair_rtt = None;
    let mut candidate_pair_avg_rtt = None;

    for report in stats.reports.values() {
        let webrtc::stats::StatsReportType::CandidatePair(pair) = report else {
            continue;
        };
        if candidate_pair_rtt.is_none() && pair.current_round_trip_time > 0.0 {
            candidate_pair_rtt = Some(pair.current_round_trip_time);
        }
        if candidate_pair_avg_rtt.is_none() {
            let avg_rtt = candidate_pair_average_rtt(pair);
            if avg_rtt > 0.0 {
                candidate_pair_avg_rtt = Some(avg_rtt);
            }
        }
        if candidate_pair_rtt.is_some() && candidate_pair_avg_rtt.is_some() {
            break;
        }
    }

    candidate_pair_rtt
        .map(|rtt| (rtt, "candidate-pair-any"))
        .or_else(|| candidate_pair_avg_rtt.map(|rtt| (rtt, "candidate-pair-any-avg")))
}

fn normalize_candidate_type(candidate_type: &impl std::fmt::Debug) -> String {
    match format!("{candidate_type:?}").to_ascii_lowercase().as_str() {
        "host" => "host".to_string(),
        "serverreflexive" => "srflx".to_string(),
        "peerreflexive" => "prflx".to_string(),
        "relay" => "relay".to_string(),
        _ => "unknown".to_string(),
    }
}
