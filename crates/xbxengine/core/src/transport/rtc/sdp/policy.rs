use std::collections::HashSet;

use crate::{api::runtime::XbxEngineNegotiationRuntimeConfig, XbxEngineRuntimeError};
use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::transport::rtc::recovery::policy::ScenarioPolicyResolver;

// SDP policy 只负责把 runtime negotiation 配置投影到上送服务端的文本 offer。
pub(crate) fn apply_offer_policy_contract(
    offer_sdp: &str,
    negotiation_config: &XbxEngineNegotiationRuntimeConfig,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
) -> String {
    let with_video_bitrate = set_media_bitrate_as(
        offer_sdp,
        "video",
        negotiation_config.video_bitrate_kbps.max(1),
    );
    let with_audio_bitrate = set_media_bitrate_as(
        &with_video_bitrate,
        "audio",
        negotiation_config.audio_bitrate_kbps.max(1),
    );
    let with_audio_layout = if negotiation_config.force_mono_audio {
        with_audio_bitrate
    } else {
        with_audio_bitrate.replace("useinbandfec=1", "useinbandfec=1; stereo=1")
    };
    let with_video_profile = reorder_h264_payload_types_by_profile(
        &with_audio_layout,
        &negotiation_config.offer_profile,
    );
    let with_constraints =
        patch_video_fmtp_constraints(&with_video_profile, negotiation_config, session_target_type);
    dedupe_rtcp_feedback_lines(&with_constraints)
}

pub(crate) fn summarize_sdp(sdp: &str) -> String {
    format!(
        "audio={} video={} application={} len={} preview={}",
        sdp.contains("\r\nm=audio ") || sdp.starts_with("m=audio "),
        sdp.contains("\r\nm=video ") || sdp.starts_with("m=video "),
        sdp.contains("\r\nm=application ") || sdp.starts_with("m=application "),
        sdp.len(),
        sdp.replace("\r\n", " | ")
            .chars()
            .take(240)
            .collect::<String>()
    )
}

pub(crate) fn validate_local_offer_sdp(offer_sdp: &str) -> Result<(), XbxEngineRuntimeError> {
    let has_audio = offer_sdp.contains("\r\nm=audio ") || offer_sdp.starts_with("m=audio ");
    let has_video = offer_sdp.contains("\r\nm=video ") || offer_sdp.starts_with("m=video ");
    let has_application =
        offer_sdp.contains("\r\nm=application ") || offer_sdp.starts_with("m=application ");
    if has_audio && has_video && has_application {
        return Ok(());
    }
    Err(XbxEngineRuntimeError::new(format!(
        "invalidLocalOfferSdp:audio={has_audio}:video={has_video}:application={has_application}:preview={}",
        offer_sdp.replace("\r\n", " | ").chars().take(320).collect::<String>()
    )))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OfferVideoConstraintTier {
    pub(crate) max_frame_size: u32,
    pub(crate) min_bitrate_kbps: u32,
    pub(crate) start_bitrate_kbps: u32,
    pub(crate) max_bitrate_kbps: u32,
}

pub(crate) fn resolve_offer_video_constraint_tier(
    negotiation_config: &XbxEngineNegotiationRuntimeConfig,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
) -> OfferVideoConstraintTier {
    ScenarioPolicyResolver::resolve_offer_video_constraint_tier(
        negotiation_config,
        session_target_type,
    )
}

fn set_media_bitrate_as(offer_sdp: &str, media: &str, bitrate: u32) -> String {
    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;
    let media_prefix = format!("m={media}");
    let bitrate_line = format!("b=AS:{bitrate}");

    while section_start < lines.len() {
        if !lines[section_start].starts_with(&media_prefix) {
            section_start += 1;
            continue;
        }

        let mut section_end = section_start + 1;
        while section_end < lines.len() && !lines[section_end].starts_with("m=") {
            section_end += 1;
        }

        let mut replaced = false;
        for index in section_start + 1..section_end {
            if lines[index].starts_with("b=AS:") {
                lines[index] = bitrate_line.clone();
                replaced = true;
                break;
            }
        }

        if !replaced {
            let mut insert_at = section_start + 1;
            while insert_at < section_end
                && (lines[insert_at].starts_with("i=") || lines[insert_at].starts_with("c="))
            {
                insert_at += 1;
            }
            lines.insert(insert_at, bitrate_line.clone());
            section_end += 1;
        }

        section_start = section_end;
    }

    lines.join("\r\n")
}

fn patch_video_fmtp_constraints(
    offer_sdp: &str,
    negotiation_config: &XbxEngineNegotiationRuntimeConfig,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
) -> String {
    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;
    let tier = resolve_offer_video_constraint_tier(negotiation_config, session_target_type);

    while section_start < lines.len() {
        if !lines[section_start].starts_with("m=video ") {
            section_start += 1;
            continue;
        }

        let mut section_end = section_start + 1;
        while section_end < lines.len() && !lines[section_end].starts_with("m=") {
            section_end += 1;
        }

        let h264_payload_types = collect_h264_payload_types(&lines[section_start..section_end]);
        for index in section_start + 1..section_end {
            let Some(payload_type) = extract_fmtp_payload_type(&lines[index]) else {
                continue;
            };
            if !h264_payload_types.contains(payload_type) {
                continue;
            }
            lines[index] = upsert_fmtp_constraints(
                &lines[index],
                &[
                    ("x-google-min-bitrate", tier.min_bitrate_kbps.to_string()),
                    (
                        "x-google-start-bitrate",
                        tier.start_bitrate_kbps.to_string(),
                    ),
                    ("x-google-max-bitrate", tier.max_bitrate_kbps.to_string()),
                    ("max-fs", tier.max_frame_size.to_string()),
                    ("max-fr", "60".to_string()),
                ],
            );
        }

        section_start = section_end;
    }

    lines.join("\r\n")
}

fn reorder_h264_payload_types_by_profile(offer_sdp: &str, preferred_profile: &str) -> String {
    let preferred_profile = normalize_h264_profile_token(preferred_profile);
    if preferred_profile.is_empty() {
        return offer_sdp.to_string();
    }

    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;

    while section_start < lines.len() {
        if !lines[section_start].starts_with("m=video ") {
            section_start += 1;
            continue;
        }

        let mut section_end = section_start + 1;
        while section_end < lines.len() && !lines[section_end].starts_with("m=") {
            section_end += 1;
        }

        let h264_payload_types = collect_h264_payload_types(&lines[section_start..section_end]);
        if h264_payload_types.is_empty() {
            section_start = section_end;
            continue;
        }

        // 先满足显式 preset family，再在剩余候选里按 family 等级降序排序。
        let mut h264_payload_entries = lines[section_start + 1..section_end]
            .iter()
            .enumerate()
            .filter_map(|(offset, line)| {
                let payload_type = extract_fmtp_payload_type(line)?;
                if !h264_payload_types.contains(payload_type) {
                    return None;
                }
                let profile_level_id = extract_h264_profile_level_id(line)?;
                Some((
                    payload_type.to_string(),
                    h264_profile_rank(&profile_level_id),
                    matches_h264_profile_family(&profile_level_id, &preferred_profile),
                    offset,
                ))
            })
            .collect::<Vec<_>>();
        if h264_payload_entries.is_empty() {
            section_start = section_end;
            continue;
        }
        h264_payload_entries.sort_by(|left, right| {
            right
                .2
                .cmp(&left.2)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.3.cmp(&right.3))
        });
        let preferred_payload_types = h264_payload_entries
            .into_iter()
            .map(|(payload_type, _, _, _)| payload_type)
            .collect::<Vec<_>>();
        let preferred_payload_type_set = preferred_payload_types
            .iter()
            .cloned()
            .collect::<HashSet<String>>();

        let mut parts = lines[section_start]
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>();
        if parts.len() > 3 {
            let reordered = preferred_payload_types
                .iter()
                .cloned()
                .chain(
                    parts[3..]
                        .iter()
                        .filter(|payload| !preferred_payload_type_set.contains(*payload))
                        .cloned(),
                )
                .collect::<Vec<String>>();
            parts.truncate(3);
            parts.extend(reordered);
            lines[section_start] = parts.join(" ");
        }

        section_start = section_end;
    }

    lines.join("\r\n")
}

fn collect_video_payload_types(video_media_line: &str) -> HashSet<String> {
    let mut parts = video_media_line.split_whitespace();
    let _ = parts.next();
    let _ = parts.next();
    let _ = parts.next();
    parts.map(ToOwned::to_owned).collect()
}

fn collect_h264_payload_types(video_section_lines: &[String]) -> HashSet<String> {
    let Some(video_media_line) = video_section_lines.first() else {
        return HashSet::new();
    };
    let video_payload_types = collect_video_payload_types(video_media_line);
    let mut h264_payload_types = HashSet::new();
    for line in video_section_lines.iter().skip(1) {
        let Some(rest) = line.strip_prefix("a=rtpmap:") else {
            continue;
        };
        let Some(space_index) = rest.find(char::is_whitespace) else {
            continue;
        };
        let payload_type = &rest[..space_index];
        if !video_payload_types.contains(payload_type) {
            continue;
        }
        let codec_name = rest[space_index + 1..]
            .split('/')
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if codec_name == "h264" {
            h264_payload_types.insert(payload_type.to_string());
        }
    }
    h264_payload_types
}

fn normalize_h264_profile_token(profile: &str) -> String {
    let normalized = profile.trim().to_ascii_lowercase();
    normalized
        .strip_prefix("profile-level-id=")
        .unwrap_or(normalized.as_str())
        .to_string()
}

fn h264_profile_rank(profile_level_id: &str) -> u8 {
    let normalized = normalize_h264_profile_token(profile_level_id);
    if normalized.starts_with("64") {
        3
    } else if normalized.starts_with("4d") {
        2
    } else if normalized.starts_with("42e") {
        1
    } else if normalized.starts_with("420") {
        0
    } else {
        0
    }
}

fn dedupe_rtcp_feedback_lines(offer_sdp: &str) -> String {
    let mut lines = Vec::<String>::new();
    let mut seen_rtcp_fb = HashSet::<String>::new();

    for raw_line in offer_sdp.split("\r\n") {
        let line = raw_line.to_string();
        if line.starts_with("a=rtcp-fb:") && !seen_rtcp_fb.insert(line.clone()) {
            continue;
        }
        lines.push(line);
    }

    lines.join("\r\n")
}

fn matches_h264_profile_family(profile_level_id: &str, preferred_profile: &str) -> bool {
    let normalized_profile_level_id = normalize_h264_profile_token(profile_level_id);
    let normalized_preferred_profile = normalize_h264_profile_token(preferred_profile);
    !normalized_preferred_profile.is_empty()
        && normalized_profile_level_id.starts_with(&normalized_preferred_profile)
}

fn extract_h264_profile_level_id(line: &str) -> Option<String> {
    let rest = line.strip_prefix("a=fmtp:")?;
    let space_index = rest.find(char::is_whitespace)?;
    for part in rest[space_index + 1..].split(';').map(str::trim) {
        if part.is_empty() {
            continue;
        }
        let lower = part.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("profile-level-id=") {
            return Some(value.to_string());
        }
    }
    None
}

fn extract_fmtp_payload_type(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("a=fmtp:")?;
    let payload_end = rest
        .find(|character: char| character.is_whitespace())
        .unwrap_or(rest.len());
    if payload_end == 0 {
        return None;
    }
    Some(&rest[..payload_end])
}

fn upsert_fmtp_constraints(line: &str, entries: &[(&str, String)]) -> String {
    let Some(rest) = line.strip_prefix("a=fmtp:") else {
        return line.to_string();
    };
    let Some(space_index) = rest.find(char::is_whitespace) else {
        return line.to_string();
    };

    let payload_type = &rest[..space_index];
    let params = rest[space_index + 1..]
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>();

    let mut normalized = params
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();

    for (key, value) in entries {
        let pattern = format!("{key}=");
        if let Some(index) = normalized
            .iter()
            .position(|part| part.to_ascii_lowercase().starts_with(&pattern))
        {
            normalized[index] = format!("{key}={value}");
        } else {
            normalized.push(format!("{key}={value}"));
        }
    }

    format!("a=fmtp:{payload_type} {}", normalized.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_offer_sdp() -> String {
        [
            "v=0",
            "o=- 0 0 IN IP4 127.0.0.1",
            "s=-",
            "t=0 0",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111",
            "c=IN IP4 0.0.0.0",
            "a=rtpmap:111 opus/48000/2",
            "a=fmtp:111 minptime=10;useinbandfec=1",
            "a=rtcp-fb:111 transport-cc",
            "a=extmap:1 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "m=video 9 UDP/TLS/RTP/SAVPF 102 104 106 108",
            "c=IN IP4 0.0.0.0",
            "a=rtpmap:102 H264/90000",
            "a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f",
            "a=rtpmap:104 H264/90000",
            "a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
            "a=rtpmap:106 H264/90000",
            "a=fmtp:106 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d0032",
            "a=rtpmap:108 H264/90000",
            "a=fmtp:108 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "m=application 9 UDP/DTLS/SCTP webrtc-datachannel",
        ]
        .join("\r\n")
    }

    #[test]
    fn h264_profile_matching_uses_family_prefixes() {
        assert!(matches_h264_profile_family("640032", "64"));
        assert!(matches_h264_profile_family("4d0032", "4d"));
        assert!(matches_h264_profile_family(
            "profile-level-id=42e01f",
            "42e"
        ));
        assert!(!matches_h264_profile_family("640032", "4d"));
    }

    #[test]
    fn apply_offer_policy_contract_places_high_family_first_when_high_is_preferred() {
        let patched = apply_offer_policy_contract(
            &sample_offer_sdp(),
            &XbxEngineNegotiationRuntimeConfig {
                target_resolution_width: 2560,
                target_resolution_height: 1440,
                video_bitrate_kbps: 60_000,
                audio_bitrate_kbps: 192,
                force_mono_audio: false,
                prefer_ipv6: false,
                offer_profile: "64".to_string(),
                ice_policy: Default::default(),
            },
            Some(&XbxEngineTargetTypeDto::Cloud),
        );

        assert!(patched.contains("m=video 9 UDP/TLS/RTP/SAVPF 108 106 104 102"));
        assert!(patched.contains("x-google-min-bitrate=12000"));
        assert!(patched.contains("x-google-start-bitrate=25000"));
        assert!(patched.contains("x-google-max-bitrate=60000"));
        assert!(patched.contains("max-fs=14400"));
        assert!(patched.contains("max-fr=60"));
    }

    #[test]
    fn apply_offer_policy_contract_keeps_main_ahead_of_baseline_when_main_is_preferred() {
        let patched = apply_offer_policy_contract(
            &sample_offer_sdp(),
            &XbxEngineNegotiationRuntimeConfig {
                offer_profile: "4d".to_string(),
                ..Default::default()
            },
            Some(&XbxEngineTargetTypeDto::Home),
        );

        assert!(patched.contains("m=video 9 UDP/TLS/RTP/SAVPF 106 108 104 102"));
    }

    #[test]
    fn apply_offer_policy_contract_dedupes_duplicate_rtcp_feedback_lines() {
        let patched = apply_offer_policy_contract(
            concat!(
                "v=0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
                "a=rtpmap:124 H264/90000\r\n",
                "a=fmtp:124 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d0032\r\n",
                "a=rtcp-fb:124 goog-remb\r\n",
                "a=rtcp-fb:124 goog-remb\r\n",
                "a=rtcp-fb:124 transport-cc\r\n",
                "a=rtcp-fb:124 transport-cc\r\n"
            ),
            &XbxEngineNegotiationRuntimeConfig {
                offer_profile: "4d".to_string(),
                ..Default::default()
            },
            Some(&XbxEngineTargetTypeDto::Cloud),
        );

        assert_eq!(patched.matches("a=rtcp-fb:124 goog-remb").count(), 1);
        assert_eq!(patched.matches("a=rtcp-fb:124 transport-cc").count(), 1);
    }
}
