use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use rtc::peer_connection::transport::{
    CandidateConfig, CandidateHostConfig, CandidateRelayConfig, CandidateServerReflexiveConfig,
    RTCIceCandidate, RTCIceCandidateInit,
};
use rtc::peer_connection::RTCPeerConnection;
use rtc::sansio::Protocol;
use rtc::shared::ifaces::ifaces;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use stun::agent::TransactionId;
use stun::fingerprint::FINGERPRINT;
use stun::message::{Getter, Message, BINDING_REQUEST};
use stun::xoraddr::XorMappedAddress;
use url::Url;

use crate::transport::rtc::connection::builder::build_ice_servers;
use crate::transport::rtc::connection::turn_runtime::TurnRuntime;
use crate::XbxEngineRuntimeError;
use xbxengine_protocol::XbxEngineSessionDto;

const RTC_IO_PUMP_MAX_PASSES: usize = 8;
const RTC_IO_READ_BUFFER_SIZE: usize = 2_048;
const RTC_SRFLX_GATHER_TIMEOUT: Duration = Duration::from_millis(300);
const RTC_SRFLX_GATHER_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Default)]
pub(crate) struct RtcIoRuntime {
    socket_v4: Option<UdpSocket>,
    socket_v6: Option<UdpSocket>,
    local_addr_v4: Option<SocketAddr>,
    local_addr_v6: Option<SocketAddr>,
    advertised_ip: Option<IpAddr>,
    relay_runtime: Option<TurnRuntime>,
    pending_writes: VecDeque<TaggedBytesMut>,
}

impl RtcIoRuntime {
    pub(crate) fn rebuild(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.stop_relay();
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

    pub(crate) fn gather_local_candidates(
        &mut self,
        session: &XbxEngineSessionDto,
    ) -> Result<Vec<RTCIceCandidateInit>, XbxEngineRuntimeError> {
        let host_candidate = self.local_candidate()?;
        let mut candidates = vec![host_candidate.clone()];
        if cfg!(test) {
            return Ok(candidates);
        }

        let Some(socket_v4) = self.socket_v4.as_ref() else {
            return Ok(candidates);
        };
        let Some(local_addr_v4) = self.local_addr_v4 else {
            return Ok(candidates);
        };
        let Some(advertised_ip) = self.advertised_ip else {
            return Ok(candidates);
        };

        for server in collect_srflx_probe_urls(session) {
            let Some(server_addr) = resolve_udp_server_addr(&server) else {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc-connection] srflx gather resolve skipped url={} host={} port={}",
                    server.raw_url,
                    server.host,
                    server.port,
                );
                continue;
            };
            match self.query_srflx_candidate(
                socket_v4,
                local_addr_v4,
                advertised_ip,
                server_addr,
                &server.raw_url,
            ) {
                Ok(Some(candidate)) => {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc-connection] srflx candidate gathered url={} candidate={}",
                        server.raw_url,
                        candidate.candidate,
                    );
                    candidates.push(candidate);
                    break;
                }
                Ok(None) => {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc-connection] srflx gather no response url={} server={}",
                        server.raw_url,
                        server_addr,
                    );
                }
                Err(error) => {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc-connection] srflx gather failed url={} server={} error={}",
                        server.raw_url,
                        server_addr,
                        error,
                    );
                }
            }
        }

        if let Some(relay_runtime) = self.ensure_relay_runtime(session)? {
            let relay_addr = relay_runtime.local_addr();
            let relay_related_addr =
                resolve_relay_related_addr(relay_runtime.base_addr(), advertised_ip);
            let candidate = CandidateRelayConfig {
                base_config: CandidateConfig {
                    network: "udp".to_string(),
                    address: relay_addr.ip().to_string(),
                    port: relay_addr.port(),
                    component: 1,
                    ..Default::default()
                },
                rel_addr: relay_related_addr.ip().to_string(),
                rel_port: relay_related_addr.port(),
                url: Some(relay_runtime.url().to_string()),
                ..Default::default()
            }
            .new_candidate_relay()
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcRelayCandidateBuildFailed: {err}"))
            })?;
            let mut candidate_init =
                RTCIceCandidate::from(&candidate).to_json().map_err(|err| {
                    XbxEngineRuntimeError::new(format!(
                        "xbxEngineRtcRelayCandidateJsonFailed: {err}"
                    ))
                })?;
            candidate_init.sdp_mid = Some("0".to_string());
            candidate_init.sdp_mline_index = Some(0);
            crate::xbx_log_warn!(
                "[xbxengine][rtc-connection] relay candidate gathered url={} candidate={}",
                relay_runtime.url(),
                candidate_init.candidate,
            );
            candidates.push(candidate_init);
        }

        Ok(candidates)
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
                match self.send_to_peer(&message.message, &message.transport) {
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
                match self.send_to_peer(&message.message, &message.transport) {
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
            progressed |= self.read_relay(peer_connection)?;

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
        self.stop_relay();
    }

    fn stop_relay(&mut self) {
        self.relay_runtime = None;
    }

    fn send_to_peer(
        &self,
        payload: &[u8],
        transport: &TransportContext,
    ) -> Result<usize, std::io::Error> {
        if let Some(relay) = self.relay_runtime.as_ref() {
            if transport.local_addr == relay.local_addr() {
                relay.send(payload, transport.peer_addr)?;
                return Ok(payload.len());
            }
        }
        let socket = match transport.peer_addr {
            SocketAddr::V4(_) => self.socket_v4.as_ref(),
            SocketAddr::V6(_) => self.socket_v6.as_ref(),
        }
        .ok_or_else(|| {
            std::io::Error::new(
                ErrorKind::AddrNotAvailable,
                "rtc io socket unavailable for peer address family",
            )
        })?;
        socket.send_to(payload, transport.peer_addr)
    }

    fn read_relay(
        &self,
        peer_connection: &mut RTCPeerConnection,
    ) -> Result<bool, XbxEngineRuntimeError> {
        let Some(relay) = self.relay_runtime.as_ref() else {
            return Ok(false);
        };
        let packets = relay.drain_incoming();
        if packets.is_empty() {
            return Ok(false);
        }
        let mut progressed = false;
        let relay_addr = relay.local_addr();
        for packet in packets {
            peer_connection
                .handle_read(TaggedBytesMut {
                    now: Instant::now(),
                    transport: TransportContext {
                        local_addr: relay_addr,
                        peer_addr: packet.from,
                        transport_protocol: TransportProtocol::UDP,
                        ecn: None,
                    },
                    message: BytesMut::from(&packet.data[..]),
                })
                .map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineRtcHandleReadFailed: {err}"))
                })?;
            progressed = true;
        }
        Ok(progressed)
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

    fn ensure_relay_runtime(
        &mut self,
        session: &XbxEngineSessionDto,
    ) -> Result<Option<&TurnRuntime>, XbxEngineRuntimeError> {
        if self.relay_runtime.is_some() {
            return Ok(self.relay_runtime.as_ref());
        }
        if let Some(turn_server) = session.turn_server.as_ref() {
            match TurnRuntime::try_create(turn_server) {
                Ok(runtime) => {
                    self.relay_runtime = Some(runtime);
                }
                Err(error) => {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc-connection] turn relay allocation failed error={}",
                        error
                    );
                }
            }
        }
        Ok(self.relay_runtime.as_ref())
    }

    fn query_srflx_candidate(
        &self,
        socket: &UdpSocket,
        local_addr: SocketAddr,
        advertised_ip: IpAddr,
        server_addr: SocketAddr,
        raw_url: &str,
    ) -> Result<Option<RTCIceCandidateInit>, XbxEngineRuntimeError> {
        let mut request = Message::new();
        request
            .build(&[
                Box::<TransactionId>::default(),
                Box::new(BINDING_REQUEST),
                Box::new(FINGERPRINT),
            ])
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcStunRequestBuildFailed: {err}"))
            })?;
        socket.send_to(&request.raw, server_addr).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcStunRequestSendFailed: {err}"))
        })?;

        let deadline = Instant::now() + RTC_SRFLX_GATHER_TIMEOUT;
        let mut buffer = [0u8; RTC_IO_READ_BUFFER_SIZE];
        while Instant::now() < deadline {
            match socket.recv_from(&mut buffer) {
                Ok((size, peer_addr)) if peer_addr == server_addr => {
                    let mut response = Message::new();
                    response.raw = buffer[..size].to_vec();
                    response.decode().map_err(|err| {
                        XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcStunResponseDecodeFailed: {err}"
                        ))
                    })?;
                    let mut mapped_addr = XorMappedAddress::default();
                    mapped_addr.get_from(&response).map_err(|err| {
                        XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcStunMappedAddressMissing: {err}"
                        ))
                    })?;
                    if mapped_addr.ip == advertised_ip && mapped_addr.port == local_addr.port() {
                        return Ok(None);
                    }
                    let candidate = CandidateServerReflexiveConfig {
                        base_config: CandidateConfig {
                            network: "udp".to_string(),
                            address: mapped_addr.ip.to_string(),
                            port: mapped_addr.port,
                            component: 1,
                            ..Default::default()
                        },
                        rel_addr: advertised_ip.to_string(),
                        rel_port: local_addr.port(),
                        url: Some(raw_url.to_string()),
                    }
                    .new_candidate_server_reflexive()
                    .map_err(|err| {
                        XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcSrflxCandidateBuildFailed: {err}"
                        ))
                    })?;
                    let mut candidate_init =
                        RTCIceCandidate::from(&candidate).to_json().map_err(|err| {
                            XbxEngineRuntimeError::new(format!(
                                "xbxEngineRtcSrflxCandidateJsonFailed: {err}"
                            ))
                        })?;
                    candidate_init.url = Some(raw_url.to_string());
                    candidate_init.sdp_mid = Some("0".to_string());
                    candidate_init.sdp_mline_index = Some(0);
                    return Ok(Some(candidate_init));
                }
                Ok((_size, _peer_addr)) => {
                    continue;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(RTC_SRFLX_GATHER_POLL_INTERVAL);
                }
                Err(err) => {
                    return Err(XbxEngineRuntimeError::new(format!(
                        "xbxEngineRtcStunResponseReadFailed: {err}"
                    )));
                }
            }
        }

        Ok(None)
    }
}

fn collect_srflx_probe_urls(session: &XbxEngineSessionDto) -> Vec<IceServerReference> {
    let mut urls = Vec::new();
    for server in build_ice_servers(session) {
        for raw_url in server.urls {
            if let Some(parsed) = parse_ice_server_url(&raw_url) {
                urls.push(parsed);
            }
        }
    }
    urls
}

fn resolve_udp_server_addr(reference: &IceServerReference) -> Option<SocketAddr> {
    let host = if reference.host.contains(':') && !reference.host.starts_with('[') {
        format!("[{}]:{}", reference.host, reference.port)
    } else {
        format!("{}:{}", reference.host, reference.port)
    };
    host.to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)
}

fn resolve_relay_related_addr(base_addr: SocketAddr, advertised_ip: IpAddr) -> SocketAddr {
    let ip = if base_addr.ip().is_unspecified() {
        advertised_ip
    } else {
        base_addr.ip()
    };
    SocketAddr::new(ip, base_addr.port())
}

fn parse_ice_server_url(raw_url: &str) -> Option<IceServerReference> {
    let parsed = parse_ice_server_url_as_url(raw_url)?;
    let scheme = parsed.scheme().to_lowercase();
    if scheme != "stun" && scheme != "turn" {
        return None;
    }
    let transport = parsed
        .query_pairs()
        .find(|(key, _)| key == "transport")
        .map(|(_, value): (_, _)| value.to_string())
        .unwrap_or_else(|| "udp".to_string());
    if transport != "udp" {
        return None;
    }
    let host = parsed.host_str()?.to_string();
    let port = parsed.port_or_known_default()?;
    Some(IceServerReference {
        raw_url: raw_url.to_string(),
        host,
        port,
    })
}

fn parse_ice_server_url_as_url(raw_url: &str) -> Option<Url> {
    if raw_url.contains("://") {
        return Url::parse(raw_url).ok();
    }
    let scheme_end = raw_url.find(':')?;
    let mut normalized = raw_url.to_string();
    normalized.replace_range(scheme_end..scheme_end + 1, "://");
    Url::parse(&normalized).ok()
}

struct IceServerReference {
    raw_url: String,
    host: String,
    port: u16,
}

fn discover_advertised_ip() -> Option<IpAddr> {
    // 对齐 webrtc-rs 的 local_interfaces 语义：先取本机接口的非 loopback 地址。
    let mut candidates = discover_local_interface_ips();
    if let Some(probe_ip) = discover_default_route_ip() {
        if advertised_ip_priority(probe_ip).is_some() && !candidates.contains(&probe_ip) {
            candidates.push(probe_ip);
        }
    }
    choose_preferred_advertised_ip(candidates)
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
        // 避免把 ULA / link-local IPv6 当成可广播候选，
        // 否则 home 串流可能只把 fdfe:* 之类的本地伪可达地址送给远端。
        IpAddr::V6(v6)
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local() =>
        {
            None
        }
        IpAddr::V6(_) => Some(1),
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
    fn advertised_ip_priority_rejects_unique_local_ipv6() {
        let ip = IpAddr::V6("fdfe:dcba:9876::1".parse::<std::net::Ipv6Addr>().unwrap());
        assert_eq!(advertised_ip_priority(ip), None);
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

    #[test]
    fn choose_preferred_advertised_ip_prefers_private_ipv4_over_global_ipv6() {
        let chosen = choose_preferred_advertised_ip(vec![
            IpAddr::V6(
                "2408:8352:a12:20e0::e6a"
                    .parse::<std::net::Ipv6Addr>()
                    .unwrap(),
            ),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 122)),
        ]);

        assert_eq!(chosen, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 122))));
    }

    #[test]
    fn collect_srflx_probe_urls_keeps_only_udp_stun_like_urls() {
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: xbxengine_protocol::XbxEngineTargetTypeDto::Home,
            turn_server: Some(xbxengine_protocol::XbxEngineTurnServerDto {
                url: "turn:relay.example.com:3478?transport=udp".to_string(),
                username: "u".to_string(),
                credential: "p".to_string(),
            }),
        };

        let urls = collect_srflx_probe_urls(&session);

        assert!(urls.iter().all(|reference| {
            reference.raw_url.starts_with("stun:") || reference.raw_url.starts_with("turn:")
        }));
        assert!(urls
            .iter()
            .all(|reference| { reference.raw_url.contains(":3478") }));
    }

    #[test]
    fn resolve_relay_related_addr_uses_advertised_ip_for_unspecified_base_addr() {
        let related = resolve_relay_related_addr(
            "0.0.0.0:45678".parse().unwrap(),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10)),
        );
        assert_eq!(related, "192.168.0.10:45678".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn parse_ice_server_url_accepts_rfc7064_style_url() {
        let parsed = parse_ice_server_url("stun:stun.example.com:3478").unwrap();
        assert_eq!(parsed.host, "stun.example.com");
        assert_eq!(parsed.port, 3478);
    }
}
