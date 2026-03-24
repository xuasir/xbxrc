use xbxengine_protocol::XbxEngineIceCandidateDto;

pub(crate) fn is_end_of_candidates_marker(sdp: &str) -> bool {
    sdp.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.eq_ignore_ascii_case("a=end-of-candidates")
            || trimmed.eq_ignore_ascii_case("end-of-candidates")
    })
}

pub(crate) fn extract_local_candidates_from_offer_sdp(
    offer_sdp: &str,
) -> Vec<XbxEngineIceCandidateDto> {
    let mut candidates = Vec::new();
    let mut current_mid: Option<String> = None;
    let mut current_mline_index: Option<u16> = None;

    for line in offer_sdp.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("m=") {
            current_mline_index =
                Some(current_mline_index.map_or(0, |index| index.saturating_add(1)));
            current_mid = None;
            continue;
        }

        if let Some(mid) = trimmed
            .strip_prefix("a=mid:")
            .or_else(|| trimmed.strip_prefix("mid:"))
        {
            let mid = mid.trim();
            if !mid.is_empty() {
                current_mid = Some(mid.to_string());
            }
            continue;
        }

        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("a=end-of-candidates")
            || trimmed.eq_ignore_ascii_case("end-of-candidates")
        {
            continue;
        }

        let Some(candidate) = trimmed
            .strip_prefix("a=candidate:")
            .or_else(|| trimmed.strip_prefix("candidate:"))
        else {
            continue;
        };

        candidates.push(XbxEngineIceCandidateDto {
            candidate: format!("candidate:{candidate}"),
            sdp_m_line_index: Some(current_mline_index.unwrap_or(0)),
            sdp_mid: current_mid
                .clone()
                .or_else(|| current_mline_index.map(|index| index.to_string())),
        });
    }

    candidates
}
