use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StreamingTargetType {
    Home,
    Cloud,
}

impl StreamingTargetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Cloud => "cloud",
        }
    }

    pub fn from_value(value: &str) -> Self {
        if value == "home" {
            return Self::Home;
        }
        Self::Cloud
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingErrorDetails {
    pub code: Option<serde_json::Value>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamingQueueDetails {
    pub estimated_total_wait_time_in_seconds: Option<u64>,
    pub estimated_allocation_time_in_seconds: Option<u64>,
    pub estimated_provisioning_time_in_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingQueueSnapshot {
    pub details: StreamingQueueDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingAnswerPayload {
    pub sdp: String,
    pub message_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingIceCandidate {
    pub candidate: String,
    pub sdp_m_line_index: Option<u32>,
    pub sdp_mid: Option<String>,
    pub username_fragment: Option<String>,
    pub message_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingTurnServerConfig {
    pub url: String,
    pub username: String,
    pub credential: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSessionSnapshot {
    pub id: String,
    pub target_id: String,
    pub path: String,
    pub target_type: String,
    pub stream_state: Option<String>,
    pub player_state: String,
    pub queue: Option<StreamingQueueSnapshot>,
    pub error_details: Option<StreamingErrorDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingCreateSessionParams {
    pub target_type: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingGetSessionParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingCloseSessionParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeOfferParams {
    pub session_id: String,
    pub sdp: String,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeOfferResult {
    pub answer: StreamingAnswerPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeIceParams {
    pub session_id: String,
    pub candidate: Vec<StreamingIceCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeIceResult {
    pub candidates: Vec<StreamingIceCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingKeepAliveParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingKeepAliveResult {
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamingListActiveSessionsParams {
    pub target_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingListActiveSessionsResult {
    pub sessions: Vec<StreamingSessionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSessionPathPayload {
    pub session_path: String,
}

#[derive(Debug, Clone)]
pub struct StreamingConfigSnapshot {
    pub resolution: i64,
    pub preferred_game_language: String,
    pub ipv6: bool,
    pub force_region_ip: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingCloseSessionResult {
    pub closed: bool,
}
