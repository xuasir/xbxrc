//! 主机配置地址与当前远端传输的补候选合同。
use crate::policy::types::HostAddr;
use crate::session::signaling::ice::IceCandidate;
use serde_json::Value;
use std::collections::BTreeSet;
use std::net::IpAddr;

#[derive(Debug, Clone, Default)]
pub struct ConsoleConfiguration {
    pub addresses: Vec<HostAddr>,
    pub invalid_addresses: u32,
}

pub fn parse_console_configuration(value: &Value) -> ConsoleConfiguration {
    let mut config = ConsoleConfiguration::default();
    let details = value.get("serverDetails").or_else(|| {
        value
            .get("configuration")
            .and_then(|value| value.get("serverDetails"))
    });
    let Some(details) = details else {
        return config;
    };
    let mut seen = BTreeSet::new();
    for (address, port) in [
        ("ipAddress", "port"),
        ("ipV4Address", "ipV4Port"),
        ("ipV6Address", "ipV6Port"),
    ] {
        let Some(raw) = details.get(address).and_then(Value::as_str) else {
            continue;
        };
        let Ok(ip) = raw.trim().trim_matches(['[', ']']).parse::<IpAddr>() else {
            config.invalid_addresses += 1;
            continue;
        };
        if ip.is_unspecified() || ip.is_multicast() {
            config.invalid_addresses += 1;
            continue;
        }
        let declared = details
            .get(port)
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
            })
            .and_then(|value| u16::try_from(value).ok())
            .filter(|port| *port > 0);
        if details.get(port).is_some() && declared.is_none() {
            config.invalid_addresses += 1;
        }
        // 缺省端口仍保留有效主机地址，9002 的兼容来源由候选生成标记。
        let entry = HostAddr {
            ip: ip.to_string(),
            port: declared.unwrap_or(0),
        };
        if seen.insert((entry.ip.clone(), entry.port)) {
            config.addresses.push(entry);
        }
    }
    config
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTransportBinding {
    pub mid: String,
    pub index: u32,
    pub ufrag: String,
}

pub fn remote_transport_bindings(sdp: &str) -> Vec<RemoteTransportBinding> {
    let mut bundle = Vec::new();
    let mut session_ufrag = None;
    let mut sections: Vec<(bool, Option<String>, Option<String>)> = Vec::new();
    for line in sdp.lines().map(str::trim) {
        if let Some(group) = line.strip_prefix("a=group:BUNDLE ") {
            bundle = group.split_whitespace().map(str::to_owned).collect();
        }
        if let Some(media) = line.strip_prefix("m=") {
            sections.push((media.split_whitespace().nth(1) != Some("0"), None, None));
        } else if let Some(mid) = line.strip_prefix("a=mid:") {
            if let Some(section) = sections.last_mut() {
                section.1 = Some(mid.to_owned());
            }
        } else if let Some(ufrag) = line
            .strip_prefix("a=ice-ufrag:")
            .filter(|value| !value.is_empty())
        {
            if let Some(section) = sections.last_mut() {
                section.2 = Some(ufrag.to_owned());
            } else {
                session_ufrag = Some(ufrag.to_owned());
            }
        }
    }
    let bundle_master = bundle.iter().find(|mid| {
        sections
            .iter()
            .any(|(enabled, section_mid, _)| *enabled && section_mid.as_ref() == Some(*mid))
    });
    sections
        .into_iter()
        .enumerate()
        .filter_map(|(index, (enabled, mid, ufrag))| {
            let mid = mid?;
            if !enabled || (bundle.contains(&mid) && bundle_master != Some(&mid)) {
                return None;
            }
            Some(RemoteTransportBinding {
                mid,
                index: index as u32,
                ufrag: ufrag.or_else(|| session_ufrag.clone())?,
            })
        })
        .collect()
}

pub fn candidate_endpoint_key(candidate: &IceCandidate) -> Option<String> {
    let tokens: Vec<_> = candidate
        .candidate
        .trim()
        .trim_start_matches("a=")
        .split_whitespace()
        .collect();
    if tokens.len() < 8 || !tokens[0].starts_with("candidate:") {
        return None;
    }
    let ip: IpAddr = tokens[4].parse().ok()?;
    let port: u16 = tokens[5].parse().ok()?;
    Some(format!(
        "{}|{}|{}|{}|{}",
        tokens[2].to_ascii_uppercase(),
        ip,
        port,
        candidate.sdp_mid.as_deref().unwrap_or_default(),
        candidate.username_fragment.as_deref().unwrap_or_default()
    ))
}

pub fn console_candidates(
    addresses: &[HostAddr],
    binding: &RemoteTransportBinding,
    source: &str,
) -> Vec<IceCandidate> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for addr in addresses {
        let Ok(ip) = addr.ip.parse::<IpAddr>() else {
            continue;
        };
        if ip.is_unspecified() || ip.is_multicast() {
            continue;
        }
        for port in [addr.port, 9002] {
            if port == 0 || !seen.insert((ip, port)) {
                continue;
            }
            let mut foundation = 2166136261u32;
            for byte in ip.to_string().bytes() {
                foundation = (foundation ^ u32::from(byte)).wrapping_mul(16777619);
            }
            let compatibility = port == 9002 && addr.port != 9002;
            let local_preference = if compatibility { 65534u32 } else { 65535 };
            let priority = (126u32 << 24) | (local_preference << 8) | 255;
            result.push(IceCandidate {
                candidate: format!(
                    "a=candidate:console{foundation:x} 1 UDP {priority} {ip} {port} typ host"
                ),
                sdp_mid: Some(binding.mid.clone()),
                sdp_m_line_index: Some(binding.index),
                username_fragment: Some(binding.ufrag.clone()),
                source: Some(if compatibility {
                    format!("{source}:compatibility-9002")
                } else {
                    source.to_owned()
                }),
                message_type: Some("iceCandidate".into()),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn parses_only_media_addresses_and_labels_compatibility_ports() {
        let config = parse_console_configuration(
            &json!({"other":{"ipAddress":"192.0.2.99","port":9999},"serverDetails":{"ipAddress":"192.0.2.1","port":5000,"ipV4Address":"192.0.2.1","ipV4Port":5000,"ipV6Address":"[2001:db8::1]","ipV6Port":"6000"}}),
        );
        assert_eq!(config.addresses.len(), 2);
        let binding = RemoteTransportBinding {
            mid: "video".into(),
            index: 1,
            ufrag: "current".into(),
        };
        let candidates = console_candidates(&config.addresses, &binding, "session-configuration");
        assert_eq!(candidates.len(), 4);
        assert_eq!(
            candidates[1].source.as_deref(),
            Some("session-configuration:compatibility-9002")
        );
        assert!(candidates
            .iter()
            .all(
                |candidate| candidate.username_fragment.as_deref() == Some("current")
                    && candidate.sdp_m_line_index == Some(1)
            ));
    }
    #[test]
    fn binds_bundle_master_and_preserves_unbundled_transports() {
        let bindings = remote_transport_bindings("v=0\na=group:BUNDLE rejected video data\na=ice-ufrag:global\nm=audio 0 UDP/TLS/RTP/SAVPF 111\na=mid:rejected\nm=video 9 UDP/TLS/RTP/SAVPF 96\na=mid:video\na=ice-ufrag:media\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\na=mid:data\nm=audio 9 UDP/TLS/RTP/SAVPF 111\na=mid:chat\na=ice-ufrag:voice\n");
        assert_eq!(
            bindings,
            vec![
                RemoteTransportBinding {
                    mid: "video".into(),
                    index: 1,
                    ufrag: "media".into()
                },
                RemoteTransportBinding {
                    mid: "chat".into(),
                    index: 3,
                    ufrag: "voice".into()
                }
            ]
        );
    }
    #[test]
    fn invalid_port_retains_compatibility_address_and_invalid_ip_is_rejected() {
        let config = parse_console_configuration(
            &json!({"serverDetails":{"ipAddress":"0.0.0.0","port":5000,"ipV4Address":"192.0.2.2","ipV4Port":99999,"ipV6Address":"invalid"}}),
        );
        assert_eq!(config.invalid_addresses, 3);
        assert_eq!(
            config.addresses,
            vec![HostAddr {
                ip: "192.0.2.2".into(),
                port: 0
            }]
        );
    }
}
