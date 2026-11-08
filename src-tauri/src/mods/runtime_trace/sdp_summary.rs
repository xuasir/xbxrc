//! 握手 trace 只保留 codec/profile/反馈能力；SDP 地址与身份留在协议层。
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn summarize_sdp(sdp: &str) -> Value {
    let mut h264 = BTreeSet::new();
    let mut fmtp = BTreeMap::new();
    let mut feedback = BTreeMap::<String, BTreeSet<&str>>::new();
    let mut extensions = BTreeSet::new();
    let mut ssrcs = BTreeSet::new();
    let mut video = false;
    let mut has_video = false;
    let mut has_audio = false;
    let mut has_application = false;
    for line in sdp.lines().take(2048).map(str::trim) {
        if line.starts_with("m=") {
            video = line.starts_with("m=video ");
            has_video |= video;
            has_audio |= line.starts_with("m=audio ");
            has_application |= line.starts_with("m=application ");
        }
        if !video {
            continue;
        }
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            if let Some((payload, codec)) = rest.split_once(' ') {
                if codec.to_ascii_lowercase().starts_with("h264/")
                    && payload.chars().all(|ch| ch.is_ascii_digit())
                {
                    h264.insert(payload.to_owned());
                }
            }
        }
        if let Some(rest) = line.strip_prefix("a=fmtp:") {
            if let Some((payload, value)) = rest.split_once(' ') {
                fmtp.insert(payload.to_owned(), value.to_owned());
            }
        }
        if let Some(rest) = line.strip_prefix("a=rtcp-fb:") {
            if let Some((payload, value)) = rest.split_once(' ') {
                let values = feedback.entry(payload.to_owned()).or_default();
                for name in ["nack", "pli", "fir", "transport-cc", "goog-remb"] {
                    if value.split_whitespace().any(|part| part == name) {
                        values.insert(name);
                    }
                }
            }
        }
        if line.starts_with("a=extmap:") {
            for name in [
                "abs-send-time",
                "video-orientation",
                "playout-delay",
                "repaired-rid",
                "rid",
                "mid",
            ] {
                if line.contains(name) {
                    extensions.insert(name);
                }
            }
            if line.contains("transport-wide-cc") || line.contains("transport-cc") {
                extensions.insert("transport-cc");
            }
        }
        if let Some(rest) = line.strip_prefix("a=ssrc:") {
            if let Some(ssrc) = rest.split_whitespace().next() {
                ssrcs.insert(ssrc);
            }
        }
    }
    json!({ "length": sdp.len(), "hasVideo": has_video, "hasAudio": has_audio, "hasApplication": has_application,
        "h264Payloads": h264.iter().take(8).map(|payload| {
            let parameters = fmtp.get(payload).map(String::as_str).unwrap_or("");
            let field = |key: &str| parameters.split(';').find_map(|item| item.trim().split_once('=').filter(|(name, _)| *name == key).map(|(_, value)| value));
            let profile = field("profile-level-id").filter(|value| value.len() == 6 && value.chars().all(|ch| ch.is_ascii_hexdigit()));
            let mode = field("packetization-mode").filter(|value| matches!(*value, "0" | "1"));
            let mut values = feedback.get(payload).cloned().unwrap_or_default();
            values.extend(feedback.get("*").into_iter().flatten().copied());
            json!({ "payloadType": payload, "profileLevelId": profile, "packetizationMode": mode,
                "spropParameterSetsPresent": field("sprop-parameter-sets").is_some(), "rtcpFeedback": values })
        }).collect::<Vec<_>>(), "videoHeaderExtensions": extensions, "ssrcCount": ssrcs.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdp_summary_retains_feedback_and_profile_without_addresses_or_identity() {
        let sdp = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 profile-level-id=42e01f;packetization-mode=1;sprop-parameter-sets=secret\r\na=rtcp-fb:96 nack pli\r\na=rtcp-fb:96 transport-cc\r\na=extmap:2 urn:ietf:params:rtp-hdrext:transport-wide-cc-02\r\na=ssrc:99 cname:private-user\r\na=ice-pwd:private-secret\r\na=candidate:1 1 udp 1 192.168.1.2 5000 typ host\r\n";
        let summary = summarize_sdp(sdp);
        assert_eq!(summary["h264Payloads"][0]["profileLevelId"], "42e01f");
        assert_eq!(summary["ssrcCount"], 1);
        assert!(!summary.to_string().contains("private"));
        assert!(!summary.to_string().contains("192.168"));
        assert_eq!(
            summary["h264Payloads"][0]["rtcpFeedback"],
            json!(["nack", "pli", "transport-cc"])
        );
    }
}
