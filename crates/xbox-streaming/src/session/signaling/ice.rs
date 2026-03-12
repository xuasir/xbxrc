use serde::{Deserialize, Serialize};

/// 统一 ICE candidate 结构，便于不同宿主只做类型适配。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_m_line_index: Option<u32>,
    pub sdp_mid: Option<String>,
    pub username_fragment: Option<String>,
    pub message_type: Option<String>,
}

/// 候选归一化策略：清洗、重排、补齐 end-of-candidates。
#[derive(Debug, Clone, Default)]
pub struct IcePolicy {
    prefer_ipv6: bool,
}

impl IcePolicy {
    pub fn new(prefer_ipv6: bool) -> Self {
        Self { prefer_ipv6 }
    }

    pub fn normalize(&self, candidates: &[IceCandidate]) -> Vec<IceCandidate> {
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

        if self.prefer_ipv6 {
            parsed.sort_by(|left, right| {
                let left_ipv6 = left.ip.contains(':');
                let right_ipv6 = right.ip.contains(':');
                right_ipv6.cmp(&left_ipv6)
            });
        }

        let mut normalized = parsed
            .iter()
            .enumerate()
            .map(|(index, entry)| IceCandidate {
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

        normalized.push(IceCandidate {
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

#[cfg(test)]
mod tests {
    use super::{IceCandidate, IcePolicy};

    #[test]
    fn normalize_filters_invalid_and_appends_end_marker() {
        let policy = IcePolicy::new(false);
        let normalized = policy.normalize(&[
            IceCandidate {
                candidate: "a=candidate:foo 1 UDP 1234 10.0.0.1 9000 typ host".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=end-of-candidates".to_string(),
                ..Default::default()
            },
        ]);

        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].sdp_mid.as_deref(), Some("0"));
        assert_eq!(normalized[0].sdp_m_line_index, Some(0));
        assert_eq!(normalized[1].candidate, "a=end-of-candidates");
    }

    #[test]
    fn normalize_prefers_ipv6_when_enabled() {
        let policy = IcePolicy::new(true);
        let normalized = policy.normalize(&[
            IceCandidate {
                candidate: "a=candidate:foo 1 UDP 1234 10.0.0.1 9000 typ host".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=candidate:bar 1 UDP 1234 2001:db8::1 9000 typ host".to_string(),
                ..Default::default()
            },
        ]);

        assert!(
            normalized[0].candidate.contains("2001:db8::1"),
            "expected ipv6 first candidate, got {:?}",
            normalized[0]
        );
    }
}
