use xbxengine_protocol::XbxEngineIceCandidateDto;

pub(crate) fn normalize_remote_candidate(
    candidate: &XbxEngineIceCandidateDto,
) -> Option<XbxEngineIceCandidateDto> {
    let trimmed = candidate.candidate.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "a=end-of-candidates" || trimmed == "end-of-candidates" {
        return None;
    }
    if trimmed.contains("UDP") && trimmed.contains("tcptype") {
        return None;
    }

    Some(XbxEngineIceCandidateDto {
        candidate: trimmed.strip_prefix("a=").unwrap_or(trimmed).to_string(),
        sdp_m_line_index: candidate.sdp_m_line_index,
        sdp_mid: candidate.sdp_mid.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::normalize_remote_candidate;
    use xbxengine_protocol::XbxEngineIceCandidateDto;

    #[test]
    fn normalize_remote_candidate_drops_end_of_candidates_and_invalid_tcp_lines() {
        let eoc = XbxEngineIceCandidateDto {
            candidate: "a=end-of-candidates".to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
        };
        assert!(normalize_remote_candidate(&eoc).is_none());

        let invalid_tcp = XbxEngineIceCandidateDto {
            candidate: "candidate:1 1 UDP 1 0.0.0.0 9 tcptype active".to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
        };
        assert!(normalize_remote_candidate(&invalid_tcp).is_none());
    }

    #[test]
    fn normalize_remote_candidate_strips_a_prefix() {
        let candidate = XbxEngineIceCandidateDto {
            candidate: "a=candidate:1 1 udp 2130706431 127.0.0.1 60000 typ host".to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
        };
        let normalized = normalize_remote_candidate(&candidate).expect("normalized");
        assert!(normalized.candidate.starts_with("candidate:1"));
    }
}
