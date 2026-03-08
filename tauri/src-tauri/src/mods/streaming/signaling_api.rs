use crate::mods::streaming::http_client::{StreamingHttpClient, StreamingHttpError};
use crate::mods::streaming::ice_normalizer::StreamingIceNormalizer;
use crate::mods::streaming::types::{StreamingAnswerPayload, StreamingIceCandidate};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct StreamingSignalingApi {
    session_base_path: String,
    http_client: StreamingHttpClient,
    ice_normalizer: StreamingIceNormalizer,
}

impl StreamingSignalingApi {
    pub fn new(
        session_base_path: String,
        http_client: StreamingHttpClient,
        ice_normalizer: StreamingIceNormalizer,
    ) -> Self {
        Self {
            session_base_path,
            http_client,
            ice_normalizer,
        }
    }

    pub async fn send_sdp(&self, session_id: &str, sdp: &str) -> Result<(), StreamingHttpError> {
        self.http_client
            .request_json(
                "POST",
                &format!("{}/{}/sdp", self.session_base_path, session_id),
                Some(json!({
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
                })),
                &[],
            )
            .await
            .map(|_| ())
    }

    pub async fn send_chat_sdp(
        &self,
        session_id: &str,
        sdp: &str,
    ) -> Result<(), StreamingHttpError> {
        self.http_client
            .request_json(
                "POST",
                &format!("{}/{}/sdp", self.session_base_path, session_id),
                Some(json!({
                    "messageType": "offer",
                    "sdp": sdp,
                    "configuration": {
                        "isMediaStreamsChatRenegotiation": true
                    }
                })),
                &[],
            )
            .await
            .map(|_| ())
    }

    pub async fn get_sdp_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<StreamingAnswerPayload>, StreamingHttpError> {
        let value = self
            .http_client
            .request_json(
                "GET",
                &format!("{}/{}/sdp", self.session_base_path, session_id),
                None,
                &[],
            )
            .await?;

        let exchange_response = value
            .get("exchangeResponse")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if exchange_response.is_empty() {
            return Ok(None);
        }

        let payload = serde_json::from_str::<Value>(exchange_response).unwrap_or(Value::Null);
        let sdp = payload
            .get("sdp")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if sdp.is_empty() {
            return Err(StreamingHttpError {
                status: None,
                body: None,
                message: "Streaming answer SDP is missing".to_string(),
            });
        }

        Ok(Some(StreamingAnswerPayload {
            sdp,
            message_type: payload
                .get("messageType")
                .and_then(Value::as_str)
                .map(|text| text.to_string()),
        }))
    }

    pub async fn get_ice_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<Vec<StreamingIceCandidate>>, StreamingHttpError> {
        let value = self
            .http_client
            .request_json(
                "GET",
                &format!("{}/{}/ice", self.session_base_path, session_id),
                None,
                &[],
            )
            .await?;

        if value == Value::String(String::new()) {
            return Ok(None);
        }

        if let Value::String(text) = value {
            let parsed = serde_json::from_str::<Vec<StreamingIceCandidate>>(&text)
                .unwrap_or_else(|_| Vec::new());
            return Ok(Some(self.ice_normalizer.normalize(&parsed)));
        }

        let payload = value
            .get("exchangeResponse")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Vec<StreamingIceCandidate>>(text).ok())
            .unwrap_or_default();

        Ok(Some(self.ice_normalizer.normalize(&payload)))
    }

    pub async fn send_ice(
        &self,
        session_id: &str,
        ice: &[StreamingIceCandidate],
    ) -> Result<(), StreamingHttpError> {
        self.http_client
            .request_json(
                "POST",
                &format!("{}/{}/ice", self.session_base_path, session_id),
                Some(json!({
                    "messageType": "iceCandidate",
                    "candidate": ice,
                })),
                &[],
            )
            .await
            .map(|_| ())
    }
}
