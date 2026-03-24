use std::{
    collections::{HashSet, VecDeque},
    io::ErrorKind,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    time::{Duration, Instant},
};

use bytes::BytesMut;
use rtc::sansio::Protocol;
use rtc::shared::error::Error as SharedError;
use rtc::shared::{TransportContext, TransportMessage, TransportProtocol};
use rtc_turn::client::{Client, ClientConfig, Event as TurnEvent};
use url::Url;

use crate::XbxEngineRuntimeError;
use xbxengine_protocol::XbxEngineTurnServerDto;

const TURN_ALLOCATE_TIMEOUT: Duration = Duration::from_secs(3);
const TURN_PUMP_MAX_PASSES: usize = 8;

pub(crate) struct RelayPacket {
    pub(crate) data: Vec<u8>,
    pub(crate) from: SocketAddr,
}

struct OutboundPacket {
    data: Vec<u8>,
    target: SocketAddr,
}

pub(crate) struct TurnRuntime {
    base_addr: SocketAddr,
    relay_addr: SocketAddr,
    turn_url: String,
    socket: UdpSocket,
    inbound: VecDeque<RelayPacket>,
    outbound: VecDeque<OutboundPacket>,
    permitted_peers: HashSet<SocketAddr>,
    pending_permission_peers: HashSet<SocketAddr>,
    client: Client,
}

impl TurnRuntime {
    pub(crate) fn try_create(
        turn_server: &XbxEngineTurnServerDto,
    ) -> Result<Self, XbxEngineRuntimeError> {
        let server_addr = parse_turn_server_addr(&turn_server.url)?;
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnBaseBindFailed: {err}"))
        })?;
        socket.set_nonblocking(true).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnBaseNonblockingFailed: {err}"))
        })?;
        let base_addr = socket.local_addr().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnBaseLocalAddrFailed: {err}"))
        })?;

        let mut client = Client::new(ClientConfig {
            stun_serv_addr: server_addr.to_string(),
            turn_serv_addr: server_addr.to_string(),
            local_addr: base_addr,
            transport_protocol: TransportProtocol::UDP,
            username: turn_server.username.clone(),
            password: turn_server.credential.clone(),
            realm: String::new(),
            software: "xbxengine-turn".to_string(),
            rto_in_ms: 200,
        })
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnClientInitFailed: {err}"))
        })?;

        let allocate_tid = client.allocate().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnAllocateFailed: {err}"))
        })?;

        let mut runtime = Self {
            base_addr,
            relay_addr: base_addr,
            turn_url: turn_server.url.clone(),
            socket,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            permitted_peers: HashSet::new(),
            pending_permission_peers: HashSet::new(),
            client,
        };

        runtime.drive_until_allocated(allocate_tid)?;
        Ok(runtime)
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.relay_addr
    }

    pub(crate) fn base_addr(&self) -> SocketAddr {
        self.base_addr
    }

    pub(crate) fn url(&self) -> &str {
        &self.turn_url
    }

    pub(crate) fn send(
        &mut self,
        payload: &[u8],
        target: SocketAddr,
    ) -> Result<(), std::io::Error> {
        self.outbound.push_back(OutboundPacket {
            data: payload.to_vec(),
            target,
        });
        self.pump().map_err(to_io_error)?;
        Ok(())
    }

    pub(crate) fn pump(&mut self) -> Result<(), XbxEngineRuntimeError> {
        for _ in 0..TURN_PUMP_MAX_PASSES {
            let mut progressed = false;

            progressed |= self.flush_client_writes()?;
            progressed |= self.read_socket()?;
            progressed |= self.handle_events()?;

            let now = Instant::now();
            if self
                .client
                .poll_timeout()
                .is_some_and(|deadline| deadline <= now)
            {
                self.client.handle_timeout(now).map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineTurnClientTimeoutFailed: {err}"))
                })?;
                progressed = true;
            }
            progressed |= self.flush_outbound()?;
            if !progressed {
                break;
            }
        }
        Ok(())
    }

    pub(crate) fn drain_incoming(&mut self) -> Vec<RelayPacket> {
        if let Err(err) = self.pump() {
            log::warn!("[xbxengine][rtc-connection] turn relay pump failed: {err}");
        }
        self.inbound.drain(..).collect()
    }

    fn drive_until_allocated(
        &mut self,
        allocate_tid: rtc_stun::message::TransactionId,
    ) -> Result<(), XbxEngineRuntimeError> {
        let deadline = Instant::now() + TURN_ALLOCATE_TIMEOUT;
        while Instant::now() < deadline {
            self.flush_client_writes()?;
            self.read_socket()?;

            while let Some(event) = self.client.poll_event() {
                match event {
                    TurnEvent::AllocateResponse(tid, addr) if tid == allocate_tid => {
                        self.relay_addr = addr;
                        return Ok(());
                    }
                    TurnEvent::AllocateError(tid, err) if tid == allocate_tid => {
                        return Err(XbxEngineRuntimeError::new(format!(
                            "xbxEngineTurnAllocateFailed: {err}"
                        )));
                    }
                    TurnEvent::TransactionTimeout(tid) if tid == allocate_tid => {
                        return Err(XbxEngineRuntimeError::new("xbxEngineTurnAllocateTimeout"));
                    }
                    other => {
                        self.handle_event(other)?;
                    }
                }
            }
            let now = Instant::now();
            if self
                .client
                .poll_timeout()
                .is_some_and(|timeout_at| timeout_at <= now)
            {
                self.client.handle_timeout(now).map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineTurnClientTimeoutFailed: {err}"))
                })?;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(XbxEngineRuntimeError::new("xbxEngineTurnAllocateTimeout"))
    }

    fn flush_client_writes(&mut self) -> Result<bool, XbxEngineRuntimeError> {
        let mut progressed = false;
        while let Some(transmit) = self.client.poll_write() {
            self.socket
                .send_to(&transmit.message, transmit.transport.peer_addr)
                .map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineTurnSocketSendFailed: {err}"))
                })?;
            progressed = true;
        }
        Ok(progressed)
    }

    fn read_socket(&mut self) -> Result<bool, XbxEngineRuntimeError> {
        let mut progressed = false;
        let mut buffer = [0u8; 2_048];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((size, peer_addr)) => {
                    self.client
                        .handle_read(TransportMessage {
                            now: Instant::now(),
                            transport: TransportContext {
                                local_addr: self.base_addr,
                                peer_addr,
                                transport_protocol: TransportProtocol::UDP,
                                ecn: None,
                            },
                            message: BytesMut::from(&buffer[..size]),
                        })
                        .map_err(|err| {
                            XbxEngineRuntimeError::new(format!(
                                "xbxEngineTurnHandleReadFailed: {err}"
                            ))
                        })?;
                    progressed = true;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(err) => {
                    return Err(XbxEngineRuntimeError::new(format!(
                        "xbxEngineTurnSocketReadFailed: {err}"
                    )));
                }
            }
        }
        Ok(progressed)
    }

    fn handle_events(&mut self) -> Result<bool, XbxEngineRuntimeError> {
        let mut progressed = false;
        while let Some(event) = self.client.poll_event() {
            self.handle_event(event)?;
            progressed = true;
        }
        Ok(progressed)
    }

    fn handle_event(&mut self, event: TurnEvent) -> Result<(), XbxEngineRuntimeError> {
        match event {
            TurnEvent::TransactionTimeout(tid) => {
                log::warn!("[xbxengine][rtc-connection] turn transaction timeout tid={tid:?}");
            }
            TurnEvent::BindingResponse(_, _) | TurnEvent::BindingError(_, _) => {}
            TurnEvent::AllocateResponse(_, _) => {}
            TurnEvent::AllocateError(_, err) => {
                log::warn!("[xbxengine][rtc-connection] turn allocate error: {err}");
            }
            TurnEvent::CreatePermissionResponse(_, peer_addr) => {
                self.pending_permission_peers.remove(&peer_addr);
                self.permitted_peers.insert(peer_addr);
            }
            TurnEvent::CreatePermissionError(_, err) => {
                log::warn!("[xbxengine][rtc-connection] turn create permission error: {err}");
            }
            TurnEvent::DataIndicationOrChannelData(_, from, data) => {
                self.inbound.push_back(RelayPacket {
                    data: data.to_vec(),
                    from,
                });
            }
        }
        Ok(())
    }

    fn flush_outbound(&mut self) -> Result<bool, XbxEngineRuntimeError> {
        let mut progressed = false;
        let mut deferred = VecDeque::new();
        while let Some(packet) = self.outbound.pop_front() {
            if !self.permitted_peers.contains(&packet.target) {
                self.ensure_permission(packet.target)?;
                deferred.push_back(packet);
                continue;
            }
            match self
                .client
                .relay(self.relay_addr)
                .and_then(|mut relay| relay.send_to(&packet.data, packet.target))
            {
                Ok(()) => {
                    progressed = true;
                }
                Err(SharedError::ErrNoPermission) => {
                    // 权限状态可能因服务端生命周期刷新而失效；这里回退到排队并重新申请权限。
                    self.permitted_peers.remove(&packet.target);
                    self.ensure_permission(packet.target)?;
                    deferred.push_back(packet);
                }
                Err(err) => {
                    return Err(XbxEngineRuntimeError::new(format!(
                        "xbxEngineTurnRelaySendFailed: {err}"
                    )));
                }
            }
        }
        self.outbound = deferred;
        Ok(progressed)
    }

    fn ensure_permission(&mut self, peer_addr: SocketAddr) -> Result<(), XbxEngineRuntimeError> {
        if self.pending_permission_peers.contains(&peer_addr) {
            return Ok(());
        }
        self.client
            .relay(self.relay_addr)
            .and_then(|mut relay| relay.create_permission(peer_addr))
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!(
                    "xbxEngineTurnCreatePermissionFailed({peer_addr}): {err}"
                ))
            })?;
        self.pending_permission_peers.insert(peer_addr);
        Ok(())
    }
}

impl Drop for TurnRuntime {
    fn drop(&mut self) {
        if let Ok(mut relay) = self.client.relay(self.relay_addr) {
            let _ = relay.close();
        }
        let _ = self.client.close();
    }
}

fn to_io_error(err: XbxEngineRuntimeError) -> std::io::Error {
    std::io::Error::other(err.to_string())
}

fn parse_turn_server_addr(url: &str) -> Result<SocketAddr, XbxEngineRuntimeError> {
    let parsed = parse_ice_url(url)?;
    let scheme = parsed.scheme().to_lowercase();
    if scheme != "turn" && scheme != "turns" {
        return Err(XbxEngineRuntimeError::new(
            "xbxEngineTurnUrlSchemeUnsupported",
        ));
    }
    let transport = parsed
        .query_pairs()
        .find(|(key, _)| key == "transport")
        .map(|(_, value): (_, _)| value.to_string())
        .unwrap_or_else(|| "udp".to_string());
    if transport != "udp" {
        return Err(XbxEngineRuntimeError::new(
            "xbxEngineTurnTransportUnsupported",
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineTurnUrlNoHost"))?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineTurnUrlNoPort"))?;
    let address = format!("{host}:{port}");
    let mut socket_addr = address
        .to_socket_addrs()
        .map_err(|err| XbxEngineRuntimeError::new(format!("xbxEngineTurnResolveFailed: {err}")))?;
    socket_addr
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineTurnResolveEmpty"))
}

fn parse_ice_url(raw_url: &str) -> Result<Url, XbxEngineRuntimeError> {
    if raw_url.contains("://") {
        return Url::parse(raw_url).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnUrlParseFailed: {err}"))
        });
    }
    let scheme_end = raw_url
        .find(':')
        .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineTurnUrlNoScheme"))?;
    let mut normalized = raw_url.to_string();
    normalized.replace_range(scheme_end..scheme_end + 1, "://");
    Url::parse(&normalized)
        .map_err(|err| XbxEngineRuntimeError::new(format!("xbxEngineTurnUrlParseFailed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_turn_server_addr_accepts_rfc7065_udp_url() {
        let parsed = parse_turn_server_addr("turn:127.0.0.1:3478?transport=udp");
        assert!(parsed.is_ok());
    }

    #[test]
    fn parse_turn_server_addr_rejects_tcp_transport() {
        let parsed = parse_turn_server_addr("turn:127.0.0.1:3478?transport=tcp");
        assert!(parsed.is_err());
    }
}
