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

/// 候选归一化策略：清洗并做稳定排序，尽量不改写服务端原始 candidate 语义。
#[derive(Debug, Clone, Default)]
pub struct IcePolicy {
    prefer_ipv6: bool,
    allow_teredo_derivation: bool,
}

impl IcePolicy {
    pub fn new(prefer_ipv6: bool) -> Self {
        Self {
            prefer_ipv6,
            allow_teredo_derivation: false,
        }
    }

    pub fn with_teredo_ipv4_derivation(mut self, enabled: bool) -> Self {
        self.allow_teredo_derivation = enabled;
        self
    }

    pub fn normalize(&self, candidates: &[IceCandidate]) -> Vec<IceCandidate> {
        let mut parsed = Vec::new();
        let mut saw_end_of_candidates = false;

        for (original_index, candidate) in candidates.iter().enumerate() {
            let raw = candidate.candidate.trim();
            if raw.is_empty() {
                continue;
            }
            if raw.eq_ignore_ascii_case("a=end-of-candidates")
                || raw.eq_ignore_ascii_case("end-of-candidates")
            {
                saw_end_of_candidates = true;
                continue;
            }

            if let Some(entry) = parse_candidate(raw, candidate, original_index) {
                parsed.push(entry.clone());
                if self.allow_teredo_derivation {
                    parsed.extend(derive_teredo_ipv4_candidates(&entry));
                }
            }
        }

        // 候选类型优先级必须稳定：host > srflx > relay > unknown。
        // 地址族仅作为同类型内排序因子，不能跨类型覆盖优先级。
        parsed.sort_by(|left, right| {
            let type_cmp = left.kind.rank().cmp(&right.kind.rank());
            if type_cmp != std::cmp::Ordering::Equal {
                return type_cmp;
            }
            let left_ipv6 = left.ip.contains(':');
            let right_ipv6 = right.ip.contains(':');
            let family_cmp = if self.prefer_ipv6 {
                right_ipv6.cmp(&left_ipv6)
            } else {
                left_ipv6.cmp(&right_ipv6)
            };
            if family_cmp != std::cmp::Ordering::Equal {
                return family_cmp;
            }
            left.original_index.cmp(&right.original_index)
        });

        let mut normalized = parsed
            .into_iter()
            .map(|entry| entry.candidate)
            .collect::<Vec<_>>();

        if saw_end_of_candidates {
            normalized.push(IceCandidate {
                candidate: "a=end-of-candidates".to_string(),
                sdp_m_line_index: Some(0),
                sdp_mid: Some("0".to_string()),
                username_fragment: None,
                message_type: Some("iceCandidate".to_string()),
            });
        }

        normalized
    }
}

#[derive(Debug, Clone)]
struct ParsedCandidate {
    ip: String,
    kind: IceCandidateKind,
    original_index: usize,
    candidate: IceCandidate,
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

    let normalized_value = normalize_candidate_value(&parts);

    Some(ParsedCandidate {
        ip: parts[4].to_string(),
        kind: parse_candidate_kind(&parts),
        original_index,
        candidate: IceCandidate {
            candidate: format!("a={normalized_value}"),
            sdp_m_line_index: source.sdp_m_line_index.or(Some(0)),
            sdp_mid: source.sdp_mid.clone().or(Some("0".to_string())),
            username_fragment: source.username_fragment.clone(),
            message_type: source
                .message_type
                .clone()
                .or(Some("iceCandidate".to_string())),
        },
    })
}

fn normalize_candidate_value(parts: &[&str]) -> String {
    let mut normalized = parts.to_vec();
    if let Some(protocol) = normalized.get_mut(2) {
        *protocol = if protocol.eq_ignore_ascii_case("udp") {
            "UDP"
        } else if protocol.eq_ignore_ascii_case("tcp") {
            "TCP"
        } else {
            *protocol
        };
    }
    normalized.join(" ")
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
    let raw = source.candidate.candidate.trim();
    let value = raw.strip_prefix("a=").unwrap_or(raw);
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 8 {
        return Vec::new();
    }
    let foundation = parts[0].trim_start_matches("candidate:");
    let component = parts[1];
    let protocol = parts[2];
    let priority = parts[3];
    let typ_and_rest = parts[6..].join(" ");

    let Some((client_ipv4, teredo_port)) = parse_teredo_endpoint(&source.ip) else {
        return Vec::new();
    };

    let mut derived = Vec::with_capacity(2);
    for (suffix, port) in [("10", 9002u16), ("11", teredo_port)] {
        let candidate = IceCandidate {
            candidate: format!(
                "a=candidate:{}{} {} {} {} {} {} {}",
                foundation, suffix, component, protocol, priority, client_ipv4, port, typ_and_rest
            ),
            sdp_m_line_index: source.candidate.sdp_m_line_index,
            sdp_mid: source.candidate.sdp_mid.clone(),
            username_fragment: source.candidate.username_fragment.clone(),
            message_type: source.candidate.message_type.clone(),
        };
        derived.push(ParsedCandidate {
            ip: client_ipv4.to_string(),
            kind: IceCandidateKind::Host,
            original_index: source.original_index,
            candidate,
        });
    }
    derived
}

fn parse_teredo_endpoint(ip: &str) -> Option<(std::net::Ipv4Addr, u16)> {
    let address = ip.parse::<std::net::Ipv6Addr>().ok()?;
    let segments = address.segments();
    if segments[0] != 0x2001 || segments[1] != 0x0000 {
        return None;
    }

    let obfuscated_port = segments[5];
    let port = !obfuscated_port;
    let client_hi = segments[6].to_be_bytes();
    let client_lo = segments[7].to_be_bytes();
    let client_ipv4 =
        std::net::Ipv4Addr::new(!client_hi[0], !client_hi[1], !client_lo[0], !client_lo[1]);
    Some((client_ipv4, port))
}

#[cfg(test)]
mod tests {
    use super::{IceCandidate, IcePolicy};

    #[test]
    fn normalize_filters_invalid_and_preserves_existing_end_marker() {
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

        assert!(normalized[0].candidate.contains("2001:db8::1"));
    }

    #[test]
    fn normalize_prefers_ipv4_when_ipv6_preference_disabled() {
        let policy = IcePolicy::new(false);
        let normalized = policy.normalize(&[
            IceCandidate {
                candidate: "a=candidate:1 1 UDP 1 2001:db8::1 9002 typ host".to_string(),
                sdp_m_line_index: Some(0),
                sdp_mid: Some("0".to_string()),
                username_fragment: None,
                message_type: None,
            },
            IceCandidate {
                candidate: "a=candidate:2 1 UDP 1 203.0.113.10 9002 typ host".to_string(),
                sdp_m_line_index: Some(0),
                sdp_mid: Some("0".to_string()),
                username_fragment: None,
                message_type: None,
            },
        ]);

        assert!(normalized[0].candidate.contains("203.0.113.10"));
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
    fn normalize_does_not_expand_teredo_candidates_by_default() {
        let policy = IcePolicy::new(false);
        let normalized = policy.normalize(&[IceCandidate {
            candidate:
                "a=candidate:219166891 1 udp 2122255103 2001:0:14c9:d806:102b:64f4:2335:0b9c 9002 typ host"
                    .to_string(),
            ..Default::default()
        }]);

        assert_eq!(normalized.len(), 1);
        assert!(normalized[0]
            .candidate
            .contains("2001:0:14c9:d806:102b:64f4:2335:0b9c 9002 typ host"));
    }

    #[test]
    fn normalize_can_expand_teredo_candidates_when_explicitly_enabled() {
        let policy = IcePolicy::new(false).with_teredo_ipv4_derivation(true);
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

    #[test]
    fn normalize_keeps_foundation_and_priority_while_normalizing_protocol_case() {
        let policy = IcePolicy::new(false);
        let normalized = policy.normalize(&[
            IceCandidate {
                candidate: "a=candidate:foo 1 udp 9 10.0.0.1 9000 typ host".to_string(),
                ..Default::default()
            },
            IceCandidate {
                candidate: "a=candidate:bar 1 udp 7 10.0.0.2 9001 typ srflx raddr 1.1.1.1 rport 1"
                    .to_string(),
                ..Default::default()
            },
        ]);

        assert!(normalized[0]
            .candidate
            .starts_with("a=candidate:foo 1 UDP 9 10.0.0.1 9000 typ host"));
        assert!(normalized[1]
            .candidate
            .starts_with("a=candidate:bar 1 UDP 7 10.0.0.2 9001 typ srflx raddr 1.1.1.1 rport 1"));
    }

    #[test]
    fn normalize_does_not_append_end_marker_when_input_lacks_it() {
        let policy = IcePolicy::new(false);
        let normalized = policy.normalize(&[IceCandidate {
            candidate: "a=candidate:foo 1 udp 9 10.0.0.1 9000 typ host".to_string(),
            ..Default::default()
        }]);

        assert_eq!(normalized.len(), 1);
        assert_ne!(normalized[0].candidate, "a=end-of-candidates");
    }
}
