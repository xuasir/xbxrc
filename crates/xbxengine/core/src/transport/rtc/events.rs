use xbxengine_protocol::XbxEngineTransportStateDto;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum RtcConnectionLifecycleState {
    #[default]
    New,
    Connecting,
    Connected,
    Disconnected,
    Recovering,
    Failed,
    Closed,
}

impl RtcConnectionLifecycleState {
    pub(crate) fn transport_state(self) -> XbxEngineTransportStateDto {
        match self {
            Self::New => XbxEngineTransportStateDto::New,
            Self::Connecting | Self::Recovering => XbxEngineTransportStateDto::Connecting,
            Self::Connected => XbxEngineTransportStateDto::Connected,
            Self::Disconnected => XbxEngineTransportStateDto::Disconnected,
            Self::Failed => XbxEngineTransportStateDto::Failed,
            Self::Closed => XbxEngineTransportStateDto::Closed,
        }
    }

    pub(crate) fn observation_label(self) -> &'static str {
        match self {
            Self::New => "rtcConnectionNew",
            Self::Connecting => "rtcConnectionConnecting",
            Self::Connected => "rtcConnectionConnected",
            Self::Disconnected => "rtcConnectionDisconnected",
            Self::Recovering => "rtcConnectionRecovering",
            Self::Failed => "rtcConnectionFailed",
            Self::Closed => "rtcConnectionClosed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RtcTransportEvent {
    ConnectionLifecycleChanged(RtcConnectionLifecycleState),
    LocalOfferCreated,
    RemoteDescriptionApplied,
    RemoteCandidateAdded,
    TransportStopped,
}
