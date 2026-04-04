use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::XbxEngineRemoteAnswerObservation;

static REMOTE_ANSWER_OBSERVATION_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MediaKind {
    Audio,
    Video,
    Other,
}

#[derive(Clone, Debug)]
struct MediaSection {
    kind: MediaKind,
    payload_types: Vec<u8>,
    header_extensions: BTreeSet<String>,
    rtcp_feedback: Vec<RtcpFeedbackLine>,
}

#[derive(Clone, Debug)]
struct RtcpFeedbackLine {
    payload_type: String,
    value: String,
}

pub(crate) fn build_remote_answer_observation(
    answer_sdp: &str,
) -> XbxEngineRemoteAnswerObservation {
    let mut sections = Vec::<MediaSection>::new();
    let mut current_section_index: Option<usize> = None;
    let mut session_header_extensions = BTreeSet::<String>::new();
    let mut session_rtcp_feedback = Vec::<RtcpFeedbackLine>::new();
    let mut codec_by_payload_type = BTreeMap::<u8, String>::new();
    let mut profile_level_id_by_payload_type = BTreeMap::<u8, String>::new();
    let mut h264_sprop_parameter_sets_by_payload_type = BTreeMap::<u8, Vec<String>>::new();

    for raw_line in answer_sdp.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(media_section) = parse_media_section(line) {
            sections.push(media_section);
            current_section_index = Some(sections.len().saturating_sub(1));
            continue;
        }
        if current_section_index.is_none() {
            if let Some(header_extension) = parse_extmap_line(line) {
                session_header_extensions.insert(header_extension);
                continue;
            }
            if let Some(feedback) = parse_rtcp_feedback_line(line) {
                session_rtcp_feedback.push(feedback);
            }
            continue;
        }
        let Some(section_index) = current_section_index else {
            continue;
        };
        if let Some((payload_type, codec)) = parse_rtpmap_line(line) {
            codec_by_payload_type.insert(payload_type, codec);
            continue;
        }
        if let Some((payload_type, sprop_parameter_sets)) =
            parse_h264_sprop_parameter_sets_line(line)
        {
            h264_sprop_parameter_sets_by_payload_type.insert(payload_type, sprop_parameter_sets);
        }
        if let Some((payload_type, profile_level_id)) = parse_profile_level_id_line(line) {
            profile_level_id_by_payload_type.insert(payload_type, profile_level_id);
            continue;
        }
        if line.starts_with("a=fmtp:") {
            continue;
        }
        if let Some(feedback) = parse_rtcp_feedback_line(line) {
            sections[section_index].rtcp_feedback.push(feedback);
            continue;
        }
        if let Some(header_extension) = parse_extmap_line(line) {
            sections[section_index]
                .header_extensions
                .insert(header_extension);
        }
    }

    let video_section = sections
        .iter()
        .find(|section| section.kind == MediaKind::Video);
    let audio_section = sections
        .iter()
        .find(|section| section.kind == MediaKind::Audio);
    let video_payload_order = video_section
        .map(|section| section.payload_types.clone())
        .unwrap_or_default();
    let selected_video_payload_type =
        video_section.and_then(|section| section.payload_types.first().copied());
    let selected_audio_payload_type =
        audio_section.and_then(|section| section.payload_types.first().copied());

    let selected_video_mime_type = selected_video_payload_type.and_then(|payload_type| {
        codec_by_payload_type
            .get(&payload_type)
            .map(|codec| format!("video/{codec}"))
    });
    let selected_video_profile_level_id = selected_video_payload_type
        .and_then(|payload_type| profile_level_id_by_payload_type.get(&payload_type).cloned());
    let selected_video_h264_sprop_parameter_sets =
        selected_video_payload_type.and_then(|payload_type| {
            h264_sprop_parameter_sets_by_payload_type
                .get(&payload_type)
                .cloned()
        });

    XbxEngineRemoteAnswerObservation {
        observation_id: REMOTE_ANSWER_OBSERVATION_ID.fetch_add(1, Ordering::Relaxed) + 1,
        video_payload_order,
        selected_video_payload_type,
        selected_video_mime_type,
        selected_video_profile_level_id,
        selected_video_h264_sprop_parameter_sets,
        accepted_video_rtcp_feedback: video_section
            .zip(selected_video_payload_type)
            .map(|(section, payload_type)| {
                collect_feedback_for_selected_payload(section, &session_rtcp_feedback, payload_type)
            })
            .unwrap_or_default(),
        accepted_audio_rtcp_feedback: audio_section
            .zip(selected_audio_payload_type)
            .map(|(section, payload_type)| {
                collect_feedback_for_selected_payload(section, &session_rtcp_feedback, payload_type)
            })
            .unwrap_or_default(),
        accepted_video_header_extensions: video_section
            .map(|section| collect_header_extensions(section, &session_header_extensions))
            .unwrap_or_default(),
        accepted_audio_header_extensions: audio_section
            .map(|section| collect_header_extensions(section, &session_header_extensions))
            .unwrap_or_default(),
        observed_at_ms: now_ms_f64(),
    }
}

fn parse_media_section(line: &str) -> Option<MediaSection> {
    let value = line.strip_prefix("m=")?;
    let mut tokens = value.split_whitespace();
    let kind = match tokens.next()? {
        "audio" => MediaKind::Audio,
        "video" => MediaKind::Video,
        _ => MediaKind::Other,
    };
    let payload_types = tokens
        .skip(2)
        .filter_map(|token| token.parse::<u8>().ok())
        .collect::<Vec<_>>();
    Some(MediaSection {
        kind,
        payload_types,
        header_extensions: BTreeSet::new(),
        rtcp_feedback: Vec::new(),
    })
}

fn parse_rtpmap_line(line: &str) -> Option<(u8, String)> {
    let value = line.strip_prefix("a=rtpmap:")?;
    let mut tokens = value.split_whitespace();
    let payload_type = tokens.next()?.parse::<u8>().ok()?;
    let codec = tokens
        .next()?
        .split('/')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if codec.is_empty() {
        None
    } else {
        Some((payload_type, codec))
    }
}

fn parse_profile_level_id_line(line: &str) -> Option<(u8, String)> {
    let value = line.strip_prefix("a=fmtp:")?;
    let mut tokens = value.split_whitespace();
    let payload_type = tokens.next()?.parse::<u8>().ok()?;
    let params = tokens.collect::<Vec<_>>().join(" ");
    let profile_level_id = params.split(';').find_map(|entry| {
        let mut kv = entry.trim().splitn(2, '=');
        let key = kv.next()?.trim();
        let value = kv.next()?.trim();
        if key.eq_ignore_ascii_case("profile-level-id") {
            Some(value.to_ascii_lowercase())
        } else {
            None
        }
    })?;
    if profile_level_id.is_empty() {
        None
    } else {
        Some((payload_type, profile_level_id))
    }
}

fn parse_h264_sprop_parameter_sets_line(line: &str) -> Option<(u8, Vec<String>)> {
    let value = line.strip_prefix("a=fmtp:")?;
    let mut tokens = value.split_whitespace();
    let payload_type = tokens.next()?.parse::<u8>().ok()?;
    let params = tokens.collect::<Vec<_>>().join(" ");
    let sprop_sets = params.split(';').find_map(|entry| {
        let mut kv = entry.trim().splitn(2, '=');
        let key = kv.next()?.trim();
        let value = kv.next()?.trim();
        if key.eq_ignore_ascii_case("sprop-parameter-sets") {
            Some(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(|part| part.to_string())
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        }
    })?;
    if sprop_sets.is_empty() {
        None
    } else {
        Some((payload_type, sprop_sets))
    }
}

fn parse_rtcp_feedback_line(line: &str) -> Option<RtcpFeedbackLine> {
    let value = line.strip_prefix("a=rtcp-fb:")?;
    let mut parts = value.split_whitespace();
    let payload_type = parts.next()?.trim().to_ascii_lowercase();
    let kind = parts.next()?.trim();
    let parameter = parts.collect::<Vec<_>>().join(" ").trim().to_string();
    let value = if parameter.is_empty() {
        kind.to_ascii_lowercase()
    } else {
        format!(
            "{}:{}",
            kind.to_ascii_lowercase(),
            parameter.to_ascii_lowercase()
        )
    };
    Some(RtcpFeedbackLine {
        payload_type,
        value,
    })
}

fn parse_extmap_line(line: &str) -> Option<String> {
    let value = line.strip_prefix("a=extmap:")?;
    let mut parts = value.split_whitespace();
    let ext_id = parts.next()?.split('/').next()?.trim();
    let uri = parts.next()?.trim().to_ascii_lowercase();
    if uri.is_empty() {
        None
    } else {
        Some(format!("{uri}#{ext_id}"))
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn collect_feedback_for_selected_payload(
    section: &MediaSection,
    session_feedback: &[RtcpFeedbackLine],
    selected_payload_type: u8,
) -> Vec<String> {
    let selected_payload_type = selected_payload_type.to_string();
    let mut accepted = BTreeSet::<String>::new();
    for feedback in session_feedback.iter().chain(section.rtcp_feedback.iter()) {
        if feedback.payload_type == "*" || feedback.payload_type == selected_payload_type {
            accepted.insert(feedback.value.clone());
        }
    }
    accepted.into_iter().collect()
}

fn collect_header_extensions(
    section: &MediaSection,
    session_header_extensions: &BTreeSet<String>,
) -> Vec<String> {
    let mut accepted = session_header_extensions.clone();
    accepted.extend(section.header_extensions.iter().cloned());
    accepted.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::build_remote_answer_observation;

    #[test]
    fn build_remote_answer_observation_extracts_selected_payload_and_feedback() {
        let answer = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
            "a=extmap:2 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtcp-fb:111 transport-cc\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 124 97 125 116\r\n",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=fmtp:124 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d002a;sprop-parameter-sets=Z2QAKqzShEQmhAAAAwAEAAADAMo8SJYR,aOhDjxMhMA==\r\n",
            "a=rtcp-fb:124 goog-remb\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
            "a=rtcp-fb:124 nack pli\r\n",
            "a=rtpmap:97 rtx/90000\r\n",
            "a=fmtp:97 apt=124\r\n",
            "a=rtpmap:125 red/90000\r\n",
            "a=rtpmap:116 ulpfec/90000\r\n",
        );

        let observation = build_remote_answer_observation(answer);
        assert_eq!(observation.video_payload_order, vec![124, 97, 125, 116]);
        assert_eq!(observation.selected_video_payload_type, Some(124));
        assert_eq!(
            observation.selected_video_mime_type.as_deref(),
            Some("video/h264")
        );
        assert_eq!(
            observation.selected_video_profile_level_id.as_deref(),
            Some("4d002a")
        );
        assert_eq!(
            observation.selected_video_h264_sprop_parameter_sets,
            Some(vec![
                "Z2QAKqzShEQmhAAAAwAEAAADAMo8SJYR".to_string(),
                "aOhDjxMhMA==".to_string()
            ])
        );
        assert!(observation
            .accepted_video_rtcp_feedback
            .iter()
            .any(|value| value == "goog-remb"));
        assert!(observation
            .accepted_video_rtcp_feedback
            .iter()
            .any(|value| value == "transport-cc"));
        assert!(observation
            .accepted_video_rtcp_feedback
            .iter()
            .any(|value| value == "nack:pli"));
        assert!(observation
            .accepted_video_header_extensions
            .iter()
            .any(|value| value.contains("transport-wide-cc")));
        assert!(observation
            .accepted_audio_rtcp_feedback
            .iter()
            .any(|value| value == "transport-cc"));
    }

    #[test]
    fn build_remote_answer_observation_merges_session_extmap_and_filters_feedback_to_selected_payload(
    ) {
        let answer = concat!(
            "v=0\r\n",
            "a=extmap:9 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 0\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtcp-fb:* transport-cc\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=fmtp:124 profile-level-id=4d002a\r\n",
            "a=rtpmap:97 rtx/90000\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
            "a=rtcp-fb:97 goog-remb\r\n",
            "a=extmap:3 urn:ietf:params:rtp-hdrext:sdes:mid\r\n",
        );

        let observation = build_remote_answer_observation(answer);
        assert!(observation
            .accepted_video_header_extensions
            .iter()
            .any(|value| value.ends_with("#9")));
        assert!(observation
            .accepted_video_header_extensions
            .iter()
            .any(|value| value.ends_with("#3")));
        assert!(observation
            .accepted_video_rtcp_feedback
            .iter()
            .any(|value| value == "transport-cc"));
        assert!(!observation
            .accepted_video_rtcp_feedback
            .iter()
            .any(|value| value == "goog-remb"));
    }
}
