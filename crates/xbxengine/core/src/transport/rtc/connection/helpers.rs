use rtc::peer_connection::transport::RTCIceCandidateInit;
use rtc::peer_connection::RTCPeerConnection;
use xbxengine_protocol::XbxEngineIceCandidateDto;

use crate::transport::rtc::connection::runtime_state::RtcIceCandidateKind;
use crate::transport::rtc::events::RtcConnectionLifecycleState;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, DataChannelLabelFact};
use crate::XbxEngineRuntimeError;

pub(crate) fn map_data_channel_label_fact(label: &str) -> Option<DataChannelLabelFact> {
    match label {
        "control" => Some(DataChannelLabelFact::Control),
        "message" => Some(DataChannelLabelFact::Message),
        "input" => Some(DataChannelLabelFact::Input),
        "chat" => Some(DataChannelLabelFact::Chat),
        _ => None,
    }
}

pub(crate) fn map_connection_lifecycle_state_fact(
    state: RtcConnectionLifecycleState,
) -> ConnectionLifecycleStateFact {
    match state {
        RtcConnectionLifecycleState::New => ConnectionLifecycleStateFact::New,
        RtcConnectionLifecycleState::Connecting => ConnectionLifecycleStateFact::Connecting,
        RtcConnectionLifecycleState::Connected => ConnectionLifecycleStateFact::Connected,
        RtcConnectionLifecycleState::Disconnected => ConnectionLifecycleStateFact::Disconnected,
        RtcConnectionLifecycleState::Recovering => ConnectionLifecycleStateFact::Recovering,
        RtcConnectionLifecycleState::Failed => ConnectionLifecycleStateFact::Failed,
        RtcConnectionLifecycleState::Closed => ConnectionLifecycleStateFact::Closed,
    }
}

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
    peer_connection: &mut RTCPeerConnection,
    candidate: &XbxEngineIceCandidateDto,
) -> Result<(), XbxEngineRuntimeError> {
    peer_connection
        .add_remote_candidate(dto_to_rtc_candidate(candidate))
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAddRemoteCandidateFailed: {err}"))
        })
}

pub(crate) fn short_text_preview(payload: &str, max_chars: usize) -> String {
    let mut preview = payload.chars().take(max_chars).collect::<String>();
    if payload.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}
