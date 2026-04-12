use std::collections::{HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use rtc::peer_connection::transport::{
    CandidateConfig, CandidateHostConfig, CandidateRelayConfig, CandidateServerReflexiveConfig,
    RTCIceCandidate, RTCIceCandidateInit,
};
use rtc::sansio::Protocol;
use rtc::shared::ifaces::ifaces;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use rtc_stun::fingerprint::FINGERPRINT;
use rtc_stun::message::{Getter, Message, TransactionId, BINDING_REQUEST};
use rtc_stun::xoraddr::XorMappedAddress;
use url::Url;

use crate::transport::rtc::connection::builder::{build_ice_servers, ControlledPeerConnection};
use crate::transport::rtc::connection::turn_runtime::TurnRuntime;
use crate::XbxEngineRuntimeError;
use xbxengine_protocol::XbxEngineSessionDto;

const RTC_IO_PUMP_MAX_PASSES: usize = 8;
const RTC_IO_READ_BUFFER_SIZE: usize = 2_048;
const RTC_SRFLX_GATHER_TIMEOUT: Duration = Duration::from_millis(300);
const RTC_SRFLX_GATHER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const RTC_NON_FATAL_SEND_DROP_ERROR_GRACE: Duration = Duration::from_secs(3);
const RTC_NON_FATAL_SEND_DROP_MIN_COUNT: u64 = 6;

#[derive(Default)]
pub(crate) struct RtcIoRuntime {
    socket_v4: Option<UdpSocket>,
    socket_v6: Option<UdpSocket>,
    local_addr_v4: Option<SocketAddr>,
    local_addr_v6: Option<SocketAddr>,
    advertised_ips: Vec<IpAddr>,
    prefer_ipv6: bool,
    non_fatal_send_drop_window: NonFatalSendDropWindow,
    relay_runtime: Option<TurnRuntime>,
    pending_writes: VecDeque<TaggedBytesMut>,
}

#[derive(Default)]
struct NonFatalSendDropWindow {
    started_at: Option<Instant>,
    drop_count: u64,
    peers: HashSet<SocketAddr>,
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
        let mut advertised_ips = if cfg!(test) {
            vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
        } else {
            discover_advertised_ips()
        };
        let fallback_ips = [
            Some(local_addr_v4.ip()),
            local_addr_v6.map(|addr| addr.ip()),
        ];
        for fallback_ip in fallback_ips.into_iter().flatten() {
            if advertised_ip_priority(fallback_ip).is_some()
                && !advertised_ips.contains(&fallback_ip)
            {
                advertised_ips.push(fallback_ip);
            }
        }
        sort_advertised_ips_by_priority(&mut advertised_ips, self.prefer_ipv6);
        self.socket_v4 = Some(socket_v4);
        self.socket_v6 = socket_v6;
        self.local_addr_v4 = Some(local_addr_v4);
        self.local_addr_v6 = local_addr_v6;
        self.advertised_ips = advertised_ips;
        self.reset_non_fatal_send_drop_window();
        self.pending_writes.clear();
        Ok(())
    }

    pub(crate) fn set_prefer_ipv6(&mut self, prefer_ipv6: bool) {
        self.prefer_ipv6 = prefer_ipv6;
        sort_advertised_ips_by_priority(&mut self.advertised_ips, self.prefer_ipv6);
    }

    pub(crate) fn gather_local_candidates(
        &mut self,
        session: &XbxEngineSessionDto,
    ) -> Result<Vec<RTCIceCandidateInit>, XbxEngineRuntimeError> {
        let mut candidates = self.local_host_candidates()?;
        if cfg!(test) {
            return Ok(candidates);
        }

        if let (Some(socket_v4), Some(local_addr_v4), Some(advertised_ip_v4)) = (
            self.socket_v4.as_ref(),
            self.local_addr_v4,
            self.preferred_advertised_ip_for_family(false),
        ) {
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
                    advertised_ip_v4,
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
        }

        let advertised_ips_snapshot = self.advertised_ips.clone();
        if let Some(relay_runtime) = self.ensure_relay_runtime(session)? {
            let relay_addr = relay_runtime.local_addr();
            let relay_related_addr =
                resolve_relay_related_addr(relay_runtime.base_addr(), &advertised_ips_snapshot);
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

    #[allow(dead_code)]
    pub(crate) fn local_candidate(&self) -> Result<RTCIceCandidateInit, XbxEngineRuntimeError> {
        self.local_host_candidates()?
            .into_iter()
            .next()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcIoLocalCandidateUnavailable"))
    }

    fn local_host_candidates(&self) -> Result<Vec<RTCIceCandidateInit>, XbxEngineRuntimeError> {
        let host_endpoints = self.local_host_endpoints();
        if host_endpoints.is_empty() {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcIoLocalCandidateUnavailable",
            ));
        }
        host_endpoints
            .into_iter()
            .map(build_host_candidate)
            .collect::<Result<Vec<_>, _>>()
    }

    fn local_host_endpoints(&self) -> Vec<SocketAddr> {
        let mut endpoints = Vec::new();
        let mut picked_v4 = false;
        let mut picked_v6 = false;

        for ip in &self.advertised_ips {
            match ip {
                IpAddr::V4(_) if !picked_v4 => {
                    if let Some(local_addr) = self.local_addr_v4 {
                        endpoints.push(SocketAddr::new(*ip, local_addr.port()));
                        picked_v4 = true;
                    }
                }
                IpAddr::V6(_) if !picked_v6 => {
                    if let Some(local_addr) = self.local_addr_v6 {
                        endpoints.push(SocketAddr::new(*ip, local_addr.port()));
                        picked_v6 = true;
                    }
                }
                _ => {}
            }
            if picked_v4 && picked_v6 {
                break;
            }
        }

        if !picked_v4 {
            if let Some(local_addr_v4) = self.local_addr_v4 {
                let ip = local_addr_v4.ip();
                if advertised_ip_priority(ip).is_some() {
                    endpoints.push(local_addr_v4);
                }
            }
        }
        if !picked_v6 {
            if let Some(local_addr_v6) = self.local_addr_v6 {
                let ip = local_addr_v6.ip();
                if advertised_ip_priority(ip).is_some() {
                    endpoints.push(local_addr_v6);
                }
            }
        }
        endpoints
    }

    fn preferred_advertised_ip_for_family(&self, ipv6: bool) -> Option<IpAddr> {
        self.advertised_ips
            .iter()
            .copied()
            .find(|ip| ip.is_ipv6() == ipv6)
    }

    fn resolve_local_addr_for_socket(&self, bind_addr: SocketAddr) -> SocketAddr {
        let advertised_ip = self.preferred_advertised_ip_for_family(bind_addr.is_ipv6());
        if let Some(advertised_ip) = advertised_ip {
            return SocketAddr::new(advertised_ip, bind_addr.port());
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

    pub(crate) fn pump(
        &mut self,
        peer_connection: &mut ControlledPeerConnection,
    ) -> Result<(), XbxEngineRuntimeError> {
        if self.socket_v4.is_none() && self.socket_v6.is_none() {
            return Ok(());
        }

        let mut had_network_progress = false;
        let mut non_fatal_drop_peers = HashSet::<SocketAddr>::new();
        let mut non_fatal_drop_count = 0u64;

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
                        had_network_progress = true;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        self.pending_writes.push_front(message);
                        break;
                    }
                    Err(err) if is_non_fatal_send_error(&err) => {
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc-connection] non-fatal send_to dropped peer={} local={} error={}",
                            message.transport.peer_addr,
                            message.transport.local_addr,
                            err
                        );
                        progressed = true;
                        non_fatal_drop_count = non_fatal_drop_count.saturating_add(1);
                        non_fatal_drop_peers.insert(message.transport.peer_addr);
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
                        had_network_progress = true;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        // 被内核写缓冲拒绝时必须保留消息，避免 poll_write 出队后静默丢包。
                        self.pending_writes.push_back(message);
                        break;
                    }
                    Err(err) if is_non_fatal_send_error(&err) => {
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc-connection] non-fatal send_to dropped peer={} local={} error={}",
                            message.transport.peer_addr,
                            message.transport.local_addr,
                            err
                        );
                        progressed = true;
                        non_fatal_drop_count = non_fatal_drop_count.saturating_add(1);
                        non_fatal_drop_peers.insert(message.transport.peer_addr);
                    }
                    Err(err) => {
                        return Err(XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcSocketSendFailed: {err}"
                        )));
                    }
                }
            }

            let v4_read = self.read_from_socket(peer_connection, false)?;
            let v6_read = self.read_from_socket(peer_connection, true)?;
            let relay_read = self.read_relay(peer_connection)?;
            progressed |= v4_read;
            progressed |= v6_read;
            progressed |= relay_read;
            had_network_progress |= v4_read || v6_read || relay_read;

            if !progressed {
                break;
            }
        }

        self.update_non_fatal_send_drop_window(
            had_network_progress,
            &non_fatal_drop_peers,
            non_fatal_drop_count,
            Instant::now(),
        )?;

        Ok(())
    }

    pub(crate) fn stop(&mut self) {
        self.socket_v4 = None;
        self.socket_v6 = None;
        self.local_addr_v4 = None;
        self.local_addr_v6 = None;
        self.advertised_ips.clear();
        self.reset_non_fatal_send_drop_window();
        self.pending_writes.clear();
        self.stop_relay();
    }

    fn reset_non_fatal_send_drop_window(&mut self) {
        self.non_fatal_send_drop_window = NonFatalSendDropWindow::default();
    }

    fn update_non_fatal_send_drop_window(
        &mut self,
        had_network_progress: bool,
        non_fatal_drop_peers: &HashSet<SocketAddr>,
        non_fatal_drop_count: u64,
        now: Instant,
    ) -> Result<(), XbxEngineRuntimeError> {
        if had_network_progress || non_fatal_drop_count == 0 || non_fatal_drop_peers.is_empty() {
            self.reset_non_fatal_send_drop_window();
            return Ok(());
        }
        let window = &mut self.non_fatal_send_drop_window;
        if window.started_at.is_none() {
            window.started_at = Some(now);
        }
        window.drop_count = window.drop_count.saturating_add(non_fatal_drop_count);
        window.peers.extend(non_fatal_drop_peers.iter().copied());

        let elapsed = now
            .duration_since(window.started_at.unwrap_or(now))
            .as_millis();
        if elapsed >= RTC_NON_FATAL_SEND_DROP_ERROR_GRACE.as_millis()
            && window.drop_count >= RTC_NON_FATAL_SEND_DROP_MIN_COUNT
        {
            let peers = window
                .peers
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineRtcAllCandidatePathsUnreachable: peers={peers} dropCount={} elapsedMs={elapsed}",
                window.drop_count
            )));
        }
        Ok(())
    }

    fn stop_relay(&mut self) {
        self.relay_runtime = None;
    }

    fn send_to_peer(
        &mut self,
        payload: &[u8],
        transport: &TransportContext,
    ) -> Result<usize, std::io::Error> {
        if let Some(relay) = self.relay_runtime.as_mut() {
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
        &mut self,
        peer_connection: &mut ControlledPeerConnection,
    ) -> Result<bool, XbxEngineRuntimeError> {
        let Some(relay) = self.relay_runtime.as_mut() else {
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
        peer_connection: &mut ControlledPeerConnection,
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
}

fn build_host_candidate(
    local_addr: SocketAddr,
) -> Result<RTCIceCandidateInit, XbxEngineRuntimeError> {
    let candidate = CandidateHostConfig {
        base_config: CandidateConfig {
            network: "udp".to_string(),
            address: local_addr.ip().to_string(),
            port: local_addr.port(),
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

fn is_non_fatal_send_error(err: &std::io::Error) -> bool {
    match err.kind() {
        ErrorKind::AddrNotAvailable
        | ErrorKind::NetworkUnreachable
        | ErrorKind::HostUnreachable => true,
        _ => matches!(err.raw_os_error(), Some(49 | 51 | 64 | 65)),
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

fn resolve_relay_related_addr(base_addr: SocketAddr, advertised_ips: &[IpAddr]) -> SocketAddr {
    let ip = if base_addr.ip().is_unspecified() {
        advertised_ips
            .iter()
            .copied()
            .find(|candidate| candidate.is_ipv6() == base_addr.is_ipv6())
            .or_else(|| advertised_ips.first().copied())
            .unwrap_or(base_addr.ip())
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

fn discover_advertised_ips() -> Vec<IpAddr> {
    // 对齐当前 RTC local_interfaces 语义：先取本机接口的非 loopback 地址。
    let mut candidates = discover_local_interface_ips();
    for probe_ip in discover_default_route_ips() {
        if advertised_ip_priority(probe_ip).is_some() && !candidates.contains(&probe_ip) {
            candidates.push(probe_ip);
        }
    }
    // `prefer_ipv6` 仅影响顺序，不影响候选集合。
    sort_advertised_ips_by_priority(&mut candidates, false);
    candidates
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

#[allow(dead_code)]
fn choose_preferred_advertised_ip(mut ips: Vec<IpAddr>) -> Option<IpAddr> {
    sort_advertised_ips_by_priority(&mut ips, false);
    ips.into_iter().next()
}

fn sort_advertised_ips_by_priority(ips: &mut Vec<IpAddr>, prefer_ipv6: bool) {
    // 先保证可达性，再处理特殊网段优先级；benchmark 网段只作为最低优先级兜底。
    ips.sort_by(|left, right| {
        let left_rank = advertised_ip_priority(*left).unwrap_or(0);
        let right_rank = advertised_ip_priority(*right).unwrap_or(0);
        let type_family_order = if prefer_ipv6 {
            right.is_ipv6().cmp(&left.is_ipv6())
        } else {
            left.is_ipv6().cmp(&right.is_ipv6())
        };
        type_family_order
            .then_with(|| right_rank.cmp(&left_rank))
            .then_with(|| left.to_string().cmp(&right.to_string()))
    });
    ips.dedup();
}

fn discover_default_route_ips() -> Vec<IpAddr> {
    const PROBES_V4: [&str; 3] = ["1.1.1.1:53", "8.8.8.8:53", "208.67.222.222:53"];
    const PROBES_V6: [&str; 2] = ["[2606:4700:4700::1111]:53", "[2001:4860:4860::8888]:53"];
    let mut discovered = Vec::new();

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
        let ip = local_addr.ip();
        if advertised_ip_priority(ip).is_some() && !discovered.contains(&ip) {
            discovered.push(ip);
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
        let ip = local_addr.ip();
        if advertised_ip_priority(ip).is_some() && !discovered.contains(&ip) {
            discovered.push(ip);
        }
    }

    discovered
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
#[path = "io_runtime.test.rs"]
mod tests;
