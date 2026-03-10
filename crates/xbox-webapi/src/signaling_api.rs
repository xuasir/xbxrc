use crate::error::WebApiError;
use crate::transport::HttpTransport;
use crate::types::IceCandidate;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdpAnswer {
    pub sdp: String,
    pub message_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SignalingApi {
    transport: HttpTransport,
    session_base_path: String,
    bearer_token: String,
}

impl SignalingApi {
    pub fn new(session_base_path: String, bearer_token: String) -> Self {
        Self {
            transport: HttpTransport::new(),
            session_base_path: session_base_path.trim_end_matches('/').to_string(),
            bearer_token,
        }
    }

    pub fn with_transport(
        transport: HttpTransport,
        session_base_path: String,
        bearer_token: String,
    ) -> Self {
        Self {
            transport,
            session_base_path: session_base_path.trim_end_matches('/').to_string(),
            bearer_token,
        }
    }

    pub async fn send_sdp(&self, session_id: &str, sdp: &str) -> Result<(), WebApiError> {
        let payload = json!({
            "messageType": "offer",
            "sdp": sdp,
            "configuration": {
                "chatConfiguration": {
                    "bytesPerSample": 2,
                    "expectedClipDurationMs": 20,
                    "format": {
                        "codec": "opus",
                        "container": "webm"
                    },
                    "numChannels": 1,
                    "sampleFrequencyHz": 24000
                },
                "chat": { "minVersion": 1, "maxVersion": 1 },
                "control": { "minVersion": 1, "maxVersion": 3 },
                "input": { "minVersion": 1, "maxVersion": 8 },
                "message": { "minVersion": 1, "maxVersion": 1 }
            }
        });
        let headers = self.create_common_headers()?;

        self.transport
            .post(
                &format!("{}/{}/sdp", self.session_base_path, session_id),
                payload,
                Some(headers),
            )
            .await
            .map(|_| ())
    }

    pub async fn send_chat_sdp(&self, session_id: &str, sdp: &str) -> Result<(), WebApiError> {
        let payload = json!({
            "messageType": "offer",
            "sdp": sdp,
            "configuration": {
                "isMediaStreamsChatRenegotiation": true
            }
        });
        let headers = self.create_common_headers()?;

        self.transport
            .post(
                &format!("{}/{}/sdp", self.session_base_path, session_id),
                payload,
                Some(headers),
            )
            .await
            .map(|_| ())
    }

    pub async fn get_sdp_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<SdpAnswer>, WebApiError> {
        let headers = self.create_common_headers()?;
        let response = self
            .transport
            .get(
                &format!("{}/{}/sdp", self.session_base_path, session_id),
                Some(headers),
            )
            .await?;

        let exchange_response = response
            .get("exchangeResponse")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if exchange_response.is_empty() {
            return Ok(None);
        }

        let payload = serde_json::from_str::<serde_json::Value>(exchange_response)
            .unwrap_or(serde_json::Value::Null);

        let sdp = payload
            .get("sdp")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WebApiError::parse("Streaming answer SDP is missing"))?
            .to_string();

        Ok(Some(SdpAnswer {
            sdp,
            message_type: payload
                .get("messageType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }))
    }

    pub async fn get_ice_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<Vec<IceCandidate>>, WebApiError> {
        let headers = self.create_common_headers()?;
        let response = self
            .transport
            .get(
                &format!("{}/{}/ice", self.session_base_path, session_id),
                Some(headers),
            )
            .await?;

        if response == serde_json::Value::String(String::new()) {
            return Ok(None);
        }

        if let serde_json::Value::String(text) = response {
            let parsed = serde_json::from_str::<Vec<IceCandidate>>(&text).unwrap_or_default();
            return Ok(Some(parsed));
        }

        let payload = response
            .get("exchangeResponse")
            .and_then(|v| v.as_str())
            .and_then(|text| serde_json::from_str::<Vec<IceCandidate>>(text).ok())
            .unwrap_or_default();

        Ok(Some(payload))
    }

    pub async fn send_ice(
        &self,
        session_id: &str,
        candidates: &[IceCandidate],
    ) -> Result<(), WebApiError> {
        let payload = json!({
            "messageType": "iceCandidate",
            "candidate": candidates,
        });
        let headers = self.create_common_headers()?;

        self.transport
            .post(
                &format!("{}/{}/ice", self.session_base_path, session_id),
                payload,
                Some(headers),
            )
            .await
            .map(|_| ())
    }

    fn authorization_header(&self) -> String {
        format!("Bearer {}", self.bearer_token)
    }

    fn create_common_headers(&self) -> Result<HeaderMap, WebApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }
}
