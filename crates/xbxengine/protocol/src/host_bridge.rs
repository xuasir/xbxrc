use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineIceCandidateDto {
    pub candidate: String,
    pub sdp_m_line_index: Option<u16>,
    pub sdp_mid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineHostRequestDto {
    ExchangeOffer {
        session_id: String,
        channel: String,
        sdp: String,
        restart: bool,
    },
    ExchangeIce {
        session_id: String,
        candidates: Vec<XbxEngineIceCandidateDto>,
        restart: bool,
    },
    KeepAliveRemoteSession {
        session_id: String,
    },
    CloseRemoteSession {
        session_id: String,
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineHostResponseDto {
    OfferExchanged {
        answer_sdp: String,
    },
    IceExchanged {
        candidates: Vec<XbxEngineIceCandidateDto>,
    },
    KeepAliveAccepted,
    RemoteSessionClosed,
}
