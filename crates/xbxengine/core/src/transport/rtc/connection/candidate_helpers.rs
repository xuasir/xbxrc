use std::collections::HashSet;
use std::net::IpAddr;

use rtc::peer_connection::transport::RTCIceCandidateInit;
use xbxengine_protocol::XbxEngineIceCandidateDto;

use crate::transport::rtc::connection::builder::ControlledPeerConnection;
use crate::transport::rtc::connection::runtime_state::RtcIceCandidateKind;
use crate::XbxEngineRuntimeError;

pub(crate) fn classify_candidate_kind(candidate: &str) -> RtcIceCandidateKind {
    let mut tokens = candidate
        .split_whitespace()
        .map(|token| token.to_ascii_lowercase());
    while let Some(token) = tokens.next() {
        if token == "typ" {
            return match tokens.next().as_deref() {
                Some("host") => RtcIceCandidateKind::Host,
                Some("srflx") => RtcIceCandidateKind::Srflx,
                Some("relay") => RtcIceCandidateKind::Relay,
                _ => RtcIceCandidateKind::Unknown,
            };
        }
    }
    RtcIceCandidateKind::Unknown
}

pub(crate) fn is_end_of_candidates_candidate(candidate: &XbxEngineIceCandidateDto) -> bool {
    let trimmed = candidate.candidate.trim();
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("a=end-of-candidates")
        || trimmed.eq_ignore_ascii_case("end-of-candidates")
}

pub(crate) fn candidate_identity_key(candidate: &XbxEngineIceCandidateDto) -> String {
    format!(
        "{}|{:?}|{}",
        candidate.candidate,
        candidate.sdp_m_line_index,
        candidate.sdp_mid.as_deref().unwrap_or("")
    )
}

pub(crate) fn dto_to_rtc_candidate(candidate: &XbxEngineIceCandidateDto) -> RTCIceCandidateInit {
    RTCIceCandidateInit {
        candidate: candidate.candidate.clone(),
        sdp_mid: candidate.sdp_mid.clone(),
        sdp_mline_index: candidate.sdp_m_line_index,
        username_fragment: None,
        url: None,
    }
}

pub(crate) fn add_remote_candidate_to_peer(
    peer_connection: &mut ControlledPeerConnection,
    candidate: &XbxEngineIceCandidateDto,
) -> Result<(), XbxEngineRuntimeError> {
    peer_connection
        .add_remote_candidate(dto_to_rtc_candidate(candidate))
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAddRemoteCandidateFailed: {err}"))
        })
}

pub(crate) fn candidate_ip_family(candidate: &XbxEngineIceCandidateDto) -> Option<bool> {
    let raw = candidate.candidate.trim();
    let value = raw.strip_prefix("a=").unwrap_or(raw);
    if !value.starts_with("candidate:") {
        return None;
    }
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 6 {
        return None;
    }
    let ip = parts[4].parse::<IpAddr>().ok()?;
    Some(ip.is_ipv6())
}

pub(crate) fn collect_candidate_ip_families(
    candidates: &[XbxEngineIceCandidateDto],
) -> HashSet<bool> {
    candidates
        .iter()
        .filter_map(candidate_ip_family)
        .collect::<HashSet<_>>()
}

pub(crate) fn should_skip_remote_candidate_for_family_mismatch(
    local_ip_families: &HashSet<bool>,
    candidate_kind: RtcIceCandidateKind,
    candidate_is_ipv6: bool,
) -> bool {
    if local_ip_families.is_empty() || local_ip_families.contains(&candidate_is_ipv6) {
        return false;
    }

    // 对 host 候选保持 4/6 对应关系，避免把单栈本地候选与异族 host 强行混用。
    // srflx/relay 仍允许跨族，交给 ICE 做最终连通性裁决。
    matches!(candidate_kind, RtcIceCandidateKind::Host)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::transport::rtc::connection::runtime_state::RtcIceCandidateKind;

    use super::should_skip_remote_candidate_for_family_mismatch;

    #[test]
    fn skips_only_host_candidates_when_family_mismatches() {
        let local_ip_families = HashSet::from([false]);

        assert!(should_skip_remote_candidate_for_family_mismatch(
            &local_ip_families,
            RtcIceCandidateKind::Host,
            true,
        ));
        assert!(!should_skip_remote_candidate_for_family_mismatch(
            &local_ip_families,
            RtcIceCandidateKind::Srflx,
            true,
        ));
        assert!(!should_skip_remote_candidate_for_family_mismatch(
            &local_ip_families,
            RtcIceCandidateKind::Relay,
            true,
        ));
    }
}
