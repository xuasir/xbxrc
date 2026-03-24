use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

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

        for (original_index, candidate) in candidates.iter().enumerate() {
            let raw = candidate.candidate.trim();
            if raw.is_empty() || raw == "a=end-of-candidates" {
                continue;
            }

            if let Some(entry) = parse_candidate(raw, candidate, original_index) {
                parsed.extend(derive_teredo_ipv4_candidates(&entry));
                parsed.push(entry);
            }
        }

        // 候选类型优先级必须稳定：host > srflx > relay > unknown。
        // IPv6 仅作为同类型内的排序因子，不能跨类型覆盖优先级。
        parsed.sort_by(|left, right| {
            let type_cmp = left.kind.rank().cmp(&right.kind.rank());
            if type_cmp != std::cmp::Ordering::Equal {
                return type_cmp;
            }
            if self.prefer_ipv6 {
                let left_ipv6 = left.ip.contains(':');
                let right_ipv6 = right.ip.contains(':');
                let ipv6_cmp = right_ipv6.cmp(&left_ipv6);
                if ipv6_cmp != std::cmp::Ordering::Equal {
                    return ipv6_cmp;
                }
            }
            left.original_index.cmp(&right.original_index)
        });

        let mut normalized = parsed
            .iter()
            .map(|entry| IceCandidate {
                // 仅补齐 `a=` 前缀并重排，不改原始候选 priority 等字段。
                candidate: format!("a={}", entry.raw_candidate),
                sdp_m_line_index: entry.sdp_m_line_index.or(Some(0)),
                sdp_mid: entry.sdp_mid.clone().or(Some("0".to_string())),
                username_fragment: entry.username_fragment.clone(),
                message_type: entry.message_type.clone(),
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
    raw_candidate: String,
    foundation: String,
    component: String,
    protocol: String,
    ip: String,
    kind: IceCandidateKind,
    sdp_m_line_index: Option<u32>,
    sdp_mid: Option<String>,
    username_fragment: Option<String>,
    message_type: Option<String>,
    original_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IceCandidateKind {
    Host,
    Srflx,
    Relay,
    Unknown,
}

impl IceCandidateKind {
    fn rank(self) -> u8 {
        match self {
            Self::Host => 0,
            Self::Srflx => 1,
            Self::Relay => 2,
            Self::Unknown => 3,
        }
    }
}

fn parse_candidate(
    raw: &str,
    source: &IceCandidate,
    original_index: usize,
) -> Option<ParsedCandidate> {
    let value = raw.strip_prefix("a=").unwrap_or(raw);
    if !value.starts_with("candidate:") {
        return None;
    }

    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 8 {
        return None;
    }
    let kind = parse_candidate_kind(&parts);

    Some(ParsedCandidate {
        raw_candidate: value.to_string(),
        foundation: parts[0].trim_start_matches("candidate:").to_string(),
        component: parts[1].to_string(),
        protocol: parts[2].to_string(),
        ip: parts[4].to_string(),
        kind,
        sdp_m_line_index: source.sdp_m_line_index,
        sdp_mid: source.sdp_mid.clone(),
        username_fragment: source.username_fragment.clone(),
        message_type: source
            .message_type
            .clone()
            .or(Some("iceCandidate".to_string())),
        original_index,
    })
}

fn parse_candidate_kind(parts: &[&str]) -> IceCandidateKind {
    for window in parts.windows(2) {
        if window[0].eq_ignore_ascii_case("typ") {
            return match window[1].to_ascii_lowercase().as_str() {
                "host" => IceCandidateKind::Host,
                "srflx" => IceCandidateKind::Srflx,
                "relay" => IceCandidateKind::Relay,
                _ => IceCandidateKind::Unknown,
            };
        }
    }
    IceCandidateKind::Unknown
}

fn derive_teredo_ipv4_candidates(source: &ParsedCandidate) -> Vec<ParsedCandidate> {
    let Some((client_ipv4, teredo_port)) = parse_teredo_endpoint(&source.ip) else {
        return Vec::new();
    };

    let mut derived = Vec::with_capacity(2);
    for (suffix, port) in [("10", 9002u16), ("11", teredo_port)] {
        let candidate = format!(
            "candidate:{}{} {} {} 1 {} {} typ host",
            source.foundation, suffix, source.component, source.protocol, client_ipv4, port
        );
        derived.push(ParsedCandidate {
            raw_candidate: candidate,
            foundation: format!("{}{}", source.foundation, suffix),
            component: source.component.clone(),
            protocol: source.protocol.clone(),
            ip: client_ipv4.to_string(),
            kind: IceCandidateKind::Host,
            sdp_m_line_index: source.sdp_m_line_index,
            sdp_mid: source.sdp_mid.clone(),
            username_fragment: source.username_fragment.clone(),
            message_type: source.message_type.clone(),
            original_index: source.original_index,
        });
    }
    derived
}

fn parse_teredo_endpoint(ip: &str) -> Option<(Ipv4Addr, u16)> {
    let address = ip.parse::<Ipv6Addr>().ok()?;
    let segments = address.segments();
    if segments[0] != 0x2001 || segments[1] != 0x0000 {
        return None;
    }

    let obfuscated_port = segments[5];
    let port = !obfuscated_port;
    let client_hi = segments[6].to_be_bytes();
    let client_lo = segments[7].to_be_bytes();
    let client_ipv4 = Ipv4Addr::new(!client_hi[0], !client_hi[1], !client_lo[0], !client_lo[1]);
    Some((client_ipv4, port))
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

    #[test]
    fn normalize_keeps_type_priority_host_srflx_relay_unknown() {
        let policy = IcePolicy::new(true);
        let normalized = policy.normalize(&[
            IceCandidate {
                candidate: "a=candidate:r 1 UDP 1 2001:db8::3 9000 typ relay".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=candidate:u 1 UDP 1 2001:db8::4 9000 typ prflx".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=candidate:s 1 UDP 1 2001:db8::2 9000 typ srflx".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=candidate:h 1 UDP 1 10.0.0.1 9000 typ host".to_string(),
                ..Default::default()
            },
        ]);

        assert!(normalized[0].candidate.contains("typ host"));
        assert!(normalized[1].candidate.contains("typ srflx"));
        assert!(normalized[2].candidate.contains("typ relay"));
        assert!(normalized[3].candidate.contains("typ prflx"));
    }

    #[test]
    fn normalize_ipv6_only_affects_same_candidate_kind() {
        let policy = IcePolicy::new(true);
        let normalized = policy.normalize(&[
            IceCandidate {
                candidate: "a=candidate:s 1 UDP 1 2001:db8::2 9000 typ srflx".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=candidate:h4 1 UDP 1 10.0.0.1 9000 typ host".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=candidate:h6 1 UDP 1 2001:db8::1 9000 typ host".to_string(),
                ..Default::default()
            },
        ]);

        assert!(normalized[0]
            .candidate
            .contains("2001:db8::1 9000 typ host"));
        assert!(normalized[1].candidate.contains("10.0.0.1 9000 typ host"));
        assert!(normalized[2].candidate.contains("typ srflx"));
    }

    #[test]
    fn normalize_adds_teredo_derived_ipv4_host_candidates() {
        let policy = IcePolicy::new(false);
        let normalized = policy.normalize(&[IceCandidate {
            candidate:
                "a=candidate:219166891 1 udp 2122255103 2001:0:14c9:d806:102b:64f4:2335:0b9c 9002 typ host"
                    .to_string(),
            ..Default::default()
        }]);

        assert!(normalized
            .iter()
            .any(|candidate| candidate.candidate.contains("220.202.244.99 9002 typ host")));
        assert!(normalized.iter().any(|candidate| candidate
            .candidate
            .contains("220.202.244.99 39691 typ host")));
    }
}
