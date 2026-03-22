use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Instant;

use bytes::BytesMut;
use rtc::peer_connection::transport::{
    CandidateConfig, CandidateHostConfig, RTCIceCandidate, RTCIceCandidateInit,
};
use rtc::peer_connection::RTCPeerConnection;
use rtc::sansio::Protocol;
use rtc::shared::ifaces::ifaces;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};

use crate::XbxEngineRuntimeError;

const RTC_IO_PUMP_MAX_PASSES: usize = 8;
const RTC_IO_READ_BUFFER_SIZE: usize = 2_048;

#[derive(Default)]
pub(crate) struct RtcIoRuntime {
    socket_v4: Option<UdpSocket>,
    socket_v6: Option<UdpSocket>,
    local_addr_v4: Option<SocketAddr>,
    local_addr_v6: Option<SocketAddr>,
    advertised_ip: Option<IpAddr>,
    pending_writes: VecDeque<TaggedBytesMut>,
}

impl RtcIoRuntime {
    pub(crate) fn rebuild(&mut self) -> Result<(), XbxEngineRuntimeError> {
        // 绑定到所有网卡，避免只暴露 loopback 候选导致云端永远不可达。
        let socket_v4 = UdpSocket::bind("0.0.0.0:0").map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcIoBindFailed: {err}"))
        })?;
        socket_v4.set_nonblocking(true).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcIoSetNonblockingFailed: {err}"))
        })?;
        let local_addr_v4 = socket_v4.local_addr().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcIoLocalAddrFailed: {err}"))
        })?;
        // IPv6 走 best-effort，避免因为 IPv6 不可用导致整体建链失败。
        let (socket_v6, local_addr_v6) = match UdpSocket::bind("[::]:0") {
            Ok(socket) => {
                socket.set_nonblocking(true).map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineRtcIoSetNonblockingFailed: {err}"))
                })?;
                let local_addr = socket.local_addr().map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineRtcIoLocalAddrFailed: {err}"))
                })?;
                (Some(socket), Some(local_addr))
            }
            Err(_) => (None, None),
        };
        let advertised_ip = if cfg!(test) {
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        } else {
            discover_advertised_ip().or_else(|| {
                let ip = local_addr_v4.ip();
                advertised_ip_priority(ip).map(|_| ip)
            })
        };
        self.socket_v4 = Some(socket_v4);
        self.socket_v6 = socket_v6;
        self.local_addr_v4 = Some(local_addr_v4);
        self.local_addr_v6 = local_addr_v6;
        self.advertised_ip = advertised_ip;
        self.pending_writes.clear();
        Ok(())
    }

    pub(crate) fn local_candidate(&self) -> Result<RTCIceCandidateInit, XbxEngineRuntimeError> {
        let local_addr_v4 = self
            .local_addr_v4
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcIoLocalCandidateUnavailable"))?;
        let advertised_ip = self
            .advertised_ip
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcIoAdvertisedIpUnavailable"))?;
        let candidate_port = match advertised_ip {
            IpAddr::V4(_) => local_addr_v4.port(),
            IpAddr::V6(_) => self
                .local_addr_v6
                .map(|local_addr| local_addr.port())
                .unwrap_or(local_addr_v4.port()),
        };
        let candidate = CandidateHostConfig {
            base_config: CandidateConfig {
                network: "udp".to_string(),
                address: advertised_ip.to_string(),
                port: candidate_port,
                component: 1,
                ..Default::default()
            },
            ..Default::default()
        }
        .new_candidate_host()
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcIoHostCandidateFailed: {err}"))
        })?;
        let mut candidate_init = RTCIceCandidate::from(&candidate).to_json().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcIoCandidateJsonFailed: {err}"))
        })?;
        candidate_init.sdp_mid = Some("0".to_string());
        candidate_init.sdp_mline_index = Some(0);
        Ok(candidate_init)
    }

    pub(crate) fn pump(
        &mut self,
        peer_connection: &mut RTCPeerConnection,
    ) -> Result<(), XbxEngineRuntimeError> {
        if self.socket_v4.is_none() && self.socket_v6.is_none() {
            return Ok(());
        }

        for _ in 0..RTC_IO_PUMP_MAX_PASSES {
            let mut progressed = false;

            while let Some(deadline) = peer_connection.poll_timeout() {
                let now = Instant::now();
                if deadline > now {
                    break;
                }
                peer_connection.handle_timeout(now).map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineRtcHandleTimeoutFailed: {err}"))
                })?;
                progressed = true;
            }

            while let Some(message) = self.pending_writes.pop_front() {
                match self.send_to_peer(&message.message, message.transport.peer_addr) {
                    Ok(_) => {
                        progressed = true;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        self.pending_writes.push_front(message);
                        break;
                    }
                    Err(err) => {
                        return Err(XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcSocketSendFailed: {err}"
                        )));
                    }
                }
            }
            if !self.pending_writes.is_empty() {
                continue;
            }

            while let Some(message) = peer_connection.poll_write() {
                match self.send_to_peer(&message.message, message.transport.peer_addr) {
                    Ok(_) => {
                        progressed = true;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        // 被内核写缓冲拒绝时必须保留消息，避免 poll_write 出队后静默丢包。
                        self.pending_writes.push_back(message);
                        break;
                    }
                    Err(err) => {
                        return Err(XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcSocketSendFailed: {err}"
                        )));
                    }
                }
            }

            progressed |= self.read_from_socket(peer_connection, false)?;
            progressed |= self.read_from_socket(peer_connection, true)?;

            if !progressed {
                break;
            }
        }

        Ok(())
    }

    pub(crate) fn stop(&mut self) {
        self.socket_v4 = None;
        self.socket_v6 = None;
        self.local_addr_v4 = None;
        self.local_addr_v6 = None;
        self.advertised_ip = None;
        self.pending_writes.clear();
    }

    fn send_to_peer(&self, payload: &[u8], peer_addr: SocketAddr) -> Result<usize, std::io::Error> {
        let socket = match peer_addr {
            SocketAddr::V4(_) => self.socket_v4.as_ref(),
            SocketAddr::V6(_) => self.socket_v6.as_ref(),
        }
        .ok_or_else(|| {
            std::io::Error::new(
                ErrorKind::AddrNotAvailable,
                "rtc io socket unavailable for peer address family",
            )
        })?;
        socket.send_to(payload, peer_addr)
    }

    fn read_from_socket(
        &self,
        peer_connection: &mut RTCPeerConnection,
        use_v6: bool,
    ) -> Result<bool, XbxEngineRuntimeError> {
        let socket = if use_v6 {
            self.socket_v6.as_ref()
        } else {
            self.socket_v4.as_ref()
        };
        let Some(socket) = socket else {
            return Ok(false);
        };
        let bind_addr = if use_v6 {
            self.local_addr_v6
        } else {
            self.local_addr_v4
        }
        .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcIoLocalAddrUnavailable"))?;
        let local_addr = self.resolve_local_addr_for_socket(bind_addr);

        let mut progressed = false;
        let mut buffer = [0u8; RTC_IO_READ_BUFFER_SIZE];
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((size, peer_addr)) => {
                    peer_connection
                        .handle_read(TaggedBytesMut {
                            now: Instant::now(),
                            transport: TransportContext {
                                local_addr,
                                peer_addr,
                                transport_protocol: TransportProtocol::UDP,
                                ecn: None,
                            },
                            message: BytesMut::from(&buffer[..size]),
                        })
                        .map_err(|err| {
                            XbxEngineRuntimeError::new(format!(
                                "xbxEngineRtcHandleReadFailed: {err}"
                            ))
                        })?;
                    progressed = true;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(err) => {
                    return Err(XbxEngineRuntimeError::new(format!(
                        "xbxEngineRtcSocketReadFailed: {err}"
                    )));
                }
            }
        }
        Ok(progressed)
    }

    fn resolve_local_addr_for_socket(&self, bind_addr: SocketAddr) -> SocketAddr {
        if let Some(advertised_ip) = self.advertised_ip {
            let same_family = matches!(
                (advertised_ip, bind_addr),
                (IpAddr::V4(_), SocketAddr::V4(_)) | (IpAddr::V6(_), SocketAddr::V6(_))
            );
            if same_family {
                return SocketAddr::new(advertised_ip, bind_addr.port());
            }
        }
        bind_addr
    }
}

fn discover_advertised_ip() -> Option<IpAddr> {
    // 对齐 webrtc-rs 的 local_interfaces 语义：先取本机接口的非 loopback 地址。
    let local_ips = discover_local_interface_ips();
    if local_ips.is_empty() {
        return discover_default_route_ip();
    }

    if let Some(probe_ip) = discover_default_route_ip() {
        if local_ips.contains(&probe_ip) && advertised_ip_priority(probe_ip).is_some() {
            return Some(probe_ip);
        }
    }

    choose_preferred_advertised_ip(local_ips)
}

fn discover_local_interface_ips() -> Vec<IpAddr> {
    let Ok(interfaces) = ifaces() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .filter_map(|iface| iface.addr.map(|addr| addr.ip()))
        .filter(|ip| advertised_ip_priority(*ip).is_some_and(|rank| rank > 0))
        .collect()
}

fn choose_preferred_advertised_ip(mut ips: Vec<IpAddr>) -> Option<IpAddr> {
    // 先保证可达性，再处理特殊网段优先级；benchmark 网段只作为最低优先级兜底。
    ips.sort_by(|left, right| {
        let left_rank = advertised_ip_priority(*left).unwrap_or(0);
        let right_rank = advertised_ip_priority(*right).unwrap_or(0);
        left_rank
            .cmp(&right_rank)
            .then_with(|| left.to_string().cmp(&right.to_string()))
    });
    ips.pop()
}

fn discover_default_route_ip() -> Option<IpAddr> {
    const PROBES_V4: [&str; 3] = ["1.1.1.1:53", "8.8.8.8:53", "208.67.222.222:53"];
    const PROBES_V6: [&str; 2] = ["[2606:4700:4700::1111]:53", "[2001:4860:4860::8888]:53"];

    for probe in PROBES_V4 {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if socket.connect(probe).is_err() {
            continue;
        }
        let Ok(local_addr) = socket.local_addr() else {
            continue;
        };
        if advertised_ip_priority(local_addr.ip()).is_some() {
            return Some(local_addr.ip());
        }
    }

    for probe in PROBES_V6 {
        let Ok(socket) = UdpSocket::bind("[::]:0") else {
            continue;
        };
        if socket.connect(probe).is_err() {
            continue;
        }
        let Ok(local_addr) = socket.local_addr() else {
            continue;
        };
        if advertised_ip_priority(local_addr.ip()).is_some() {
            return Some(local_addr.ip());
        }
    }

    None
}

fn advertised_ip_priority(ip: IpAddr) -> Option<u8> {
    // 广播给远端的地址必须是可实际到达的接口地址。
    // 198.18.0.0/15 是基准测试保留网段，不能作为可广播 ICE 候选。
    match ip {
        IpAddr::V4(v4) if v4.is_loopback() || v4.is_unspecified() => None,
        IpAddr::V4(v4) if is_benchmark_ipv4(v4) => None,
        IpAddr::V4(v4) if v4.is_private() => Some(2),
        IpAddr::V4(_) => Some(1),
        IpAddr::V6(v6) => (!v6.is_loopback() && !v6.is_unspecified()).then_some(1),
    }
}

fn is_benchmark_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 198 && (b == 18 || b == 19)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_ip_priority_rejects_loopback() {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(advertised_ip_priority(ip), None);
    }

    #[test]
    fn advertised_ip_priority_accepts_non_loopback_ipv4() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
        assert_eq!(advertised_ip_priority(ip), Some(2));
    }

    #[test]
    fn advertised_ip_priority_rejects_benchmark_ipv4() {
        let ip = IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1));
        assert_eq!(advertised_ip_priority(ip), None);
    }

    #[test]
    fn advertised_ip_priority_accepts_non_loopback_ipv6() {
        let ip = IpAddr::V6("240e:3a1:abcd::10".parse::<std::net::Ipv6Addr>().unwrap());
        assert_eq!(advertised_ip_priority(ip), Some(1));
    }

    #[test]
    fn resolve_local_addr_for_socket_keeps_bind_addr_when_family_differs() {
        let runtime = RtcIoRuntime {
            advertised_ip: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10))),
            ..Default::default()
        };
        let bind_addr: SocketAddr = "[::1]:7000".parse().unwrap();
        assert_eq!(runtime.resolve_local_addr_for_socket(bind_addr), bind_addr);
    }

    #[test]
    fn resolve_local_addr_for_socket_uses_advertised_ip_when_family_matches() {
        let runtime = RtcIoRuntime {
            advertised_ip: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10))),
            ..Default::default()
        };
        let bind_addr: SocketAddr = "0.0.0.0:7000".parse().unwrap();
        assert_eq!(
            runtime.resolve_local_addr_for_socket(bind_addr),
            "192.168.0.10:7000".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn choose_preferred_advertised_ip_prefers_private_over_benchmark() {
        let chosen = choose_preferred_advertised_ip(vec![
            IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
        ]);
        assert_eq!(chosen, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
    }
}
