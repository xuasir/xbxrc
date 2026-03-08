use crate::mods::streaming::types::StreamingIceCandidate;

#[derive(Clone)]
pub struct StreamingIceNormalizer {
    ipv6: bool,
}

impl StreamingIceNormalizer {
    pub fn new(ipv6: bool) -> Self {
        Self { ipv6 }
    }

    // 候选统一归一化，减少 renderer 对后端差异的处理成本。
    pub fn normalize(&self, candidates: &[StreamingIceCandidate]) -> Vec<StreamingIceCandidate> {
        let mut parsed = Vec::new();

        for candidate in candidates {
            let raw = candidate.candidate.trim();
            if raw.is_empty() || raw == "a=end-of-candidates" {
                continue;
            }

            if let Some(entry) = parse_candidate(raw) {
                parsed.push(entry);
            }
        }

        if self.ipv6 {
            parsed.sort_by(|left, right| {
                let left_ipv6 = left.ip.contains(':');
                let right_ipv6 = right.ip.contains(':');
                right_ipv6.cmp(&left_ipv6)
            });
        }

        let mut normalized = parsed
            .iter()
            .enumerate()
            .map(|(index, entry)| StreamingIceCandidate {
                candidate: format!(
                    "a=candidate:{} 1 UDP {} {} {} {}",
                    index + 1,
                    if index == 0 { 2_130_706_431 } else { 1 },
                    entry.ip,
                    entry.port,
                    entry.tail
                ),
                sdp_m_line_index: Some(0),
                sdp_mid: Some("0".to_string()),
                username_fragment: None,
                message_type: Some("iceCandidate".to_string()),
            })
            .collect::<Vec<_>>();

        normalized.push(StreamingIceCandidate {
            candidate: "a=end-of-candidates".to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
            username_fragment: None,
            message_type: Some("iceCandidate".to_string()),
        });

        normalized
    }
}

#[derive(Debug, Clone)]
struct ParsedCandidate {
    ip: String,
    port: String,
    tail: String,
}

fn parse_candidate(raw: &str) -> Option<ParsedCandidate> {
    let value = raw.strip_prefix("a=").unwrap_or(raw);
    if !value.starts_with("candidate:") {
        return None;
    }

    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 8 {
        return None;
    }

    Some(ParsedCandidate {
        ip: parts[4].to_string(),
        port: parts[5].to_string(),
        tail: parts[6..].join(" "),
    })
}
