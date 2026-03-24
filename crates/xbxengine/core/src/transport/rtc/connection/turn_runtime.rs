use std::{
    collections::VecDeque,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{
    net::UdpSocket as TokioUdpSocket,
    runtime::{Builder, Runtime},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};
use turn::client::{Client, ClientConfig};
use url::Url;
use webrtc_util::Conn;

use crate::XbxEngineRuntimeError;
use xbxengine_protocol::XbxEngineTurnServerDto;

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
    inbound: Arc<Mutex<VecDeque<RelayPacket>>>,
    outbound_tx: UnboundedSender<OutboundPacket>,
    send_handle: JoinHandle<()>,
    recv_handle: JoinHandle<()>,
    runtime: Arc<Runtime>,
    client: Client,
}

impl TurnRuntime {
    pub(crate) fn try_create(
        turn_server: &XbxEngineTurnServerDto,
    ) -> Result<Self, XbxEngineRuntimeError> {
        let server_addr = parse_turn_server_addr(&turn_server.url)?;
        let runtime = Arc::new(
            Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineTurnRuntimeBuildFailed: {err}"))
                })?,
        );

        let std_socket = UdpSocket::bind("0.0.0.0:0").map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnBaseBindFailed: {err}"))
        })?;
        std_socket.set_nonblocking(true).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnBaseNonblockingFailed: {err}"))
        })?;
        let tokio_socket = TokioUdpSocket::from_std(std_socket).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnBindTokioSocketFailed: {err}"))
        })?;

        let conn: Arc<dyn Conn + Send + Sync> = Arc::new(tokio_socket);
        let base_addr = conn.local_addr().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnBaseLocalAddrFailed: {err}"))
        })?;
        let config = ClientConfig {
            stun_serv_addr: server_addr.to_string(),
            turn_serv_addr: server_addr.to_string(),
            username: turn_server.username.clone(),
            password: turn_server.credential.clone(),
            realm: String::new(),
            software: "xbxengine-turn".to_string(),
            rto_in_ms: 200,
            conn: Arc::clone(&conn),
            vnet: None,
        };

        let client = runtime.block_on(Client::new(config)).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnClientInitFailed: {err}"))
        })?;
        runtime.block_on(client.listen()).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnClientListenFailed: {err}"))
        })?;
        let relay_conn = runtime.block_on(client.allocate()).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnAllocateFailed: {err}"))
        })?;
        let relay_conn: Arc<dyn Conn + Send + Sync> = Arc::new(relay_conn);
        let relay_addr = relay_conn.local_addr().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineTurnLocalAddrFailed: {err}"))
        })?;

        let inbound = Arc::new(Mutex::new(VecDeque::new()));
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();

        let send_handle = {
            let relay_conn = Arc::clone(&relay_conn);
            runtime.spawn(async move {
                let mut outbound_rx: UnboundedReceiver<OutboundPacket> = outbound_rx;
                while let Some(packet) = outbound_rx.recv().await {
                    if let Err(err) = relay_conn.send_to(&packet.data, packet.target).await {
                        log::warn!("[xbxengine][rtc-connection] turn relay send failed: {err}");
                    }
                }
            })
        };

        let recv_handle = {
            let relay_conn = Arc::clone(&relay_conn);
            let inbound = Arc::clone(&inbound);
            runtime.spawn(async move {
                let mut buffer = vec![0u8; 2_048];
                loop {
                    match relay_conn.recv_from(&mut buffer).await {
                        Ok((size, from)) => {
                            let packet = RelayPacket {
                                data: buffer[..size].to_vec(),
                                from,
                            };
                            let mut queue = inbound.lock().unwrap();
                            queue.push_back(packet);
                        }
                        Err(err) => {
                            log::warn!("[xbxengine][rtc-connection] turn relay recv failed: {err}");
                            break;
                        }
                    }
                }
            })
        };

        Ok(Self {
            base_addr,
            relay_addr,
            turn_url: turn_server.url.clone(),
            inbound,
            outbound_tx,
            send_handle,
            recv_handle,
            runtime,
            client,
        })
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

    pub(crate) fn send(&self, payload: &[u8], target: SocketAddr) -> Result<(), std::io::Error> {
        self.outbound_tx
            .send(OutboundPacket {
                data: payload.to_vec(),
                target,
            })
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "turn relay worker stopped")
            })
    }

    pub(crate) fn drain_incoming(&self) -> Vec<RelayPacket> {
        let mut queue = self.inbound.lock().unwrap();
        queue.drain(..).collect()
    }
}

impl Drop for TurnRuntime {
    fn drop(&mut self) {
        self.send_handle.abort();
        self.recv_handle.abort();
        let client = self.client.clone();
        let (close_tx, close_rx) = std::sync::mpsc::channel();
        let _close_handle = self.runtime.spawn(async move {
            let _ = client.close().await;
            let _ = close_tx.send(());
        });
        let _ = close_rx.recv_timeout(Duration::from_secs(1));
    }
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
