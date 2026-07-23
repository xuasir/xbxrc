use crate::error::WebApiError;
use crate::transport::HttpTransport;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartStreamResponse {
    pub session_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStateResponse {
    pub state: Option<String>,
    pub error_details: Option<StreamErrorDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamErrorDetails {
    pub code: Option<serde_json::Value>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WaitingTimesResponse {
    pub estimated_total_wait_time_in_seconds: Option<u64>,
    pub estimated_allocation_time_in_seconds: Option<u64>,
    pub estimated_provisioning_time_in_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleInfo {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfigResponse {
    pub title_id: String,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct SessionApi {
    transport: HttpTransport,
    target_type: String,
    base_url: String,
    bearer_token: String,
    header_overrides: BTreeMap<String, String>,
}

impl SessionApi {
    /// RFC: 执行层不再自行推导 device_info，统一消费编译产出的 headers。
    pub fn new(
        target_type: String,
        base_url: String,
        bearer_token: String,
        header_overrides: BTreeMap<String, String>,
    ) -> Self {
        Self {
            transport: HttpTransport::new(),
            target_type,
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer_token,
            header_overrides,
        }
    }

    pub fn with_transport(
        transport: HttpTransport,
        target_type: String,
        base_url: String,
        bearer_token: String,
        header_overrides: BTreeMap<String, String>,
    ) -> Self {
        Self {
            transport,
            target_type,
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer_token,
            header_overrides,
        }
    }

    pub async fn start_stream_with_payload(
        &self,
        payload: &Value,
    ) -> Result<StartStreamResponse, WebApiError> {
        let headers = self.create_headers()?;

        let response = self
            .transport
            .post(
                &self.endpoint(&format!("/v5/sessions/{}/play", self.target_type)),
                payload.clone(),
                Some(headers),
            )
            .await?;

        let session_path = response
            .get("sessionPath")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WebApiError::parse("Missing sessionPath in response"))?
            .to_string();

        Ok(StartStreamResponse { session_path })
    }

    pub async fn stop_stream(&self, session_id: &str) -> Result<(), WebApiError> {
        let headers = self.create_headers()?;
        self.transport
            .delete(
                &self.endpoint(&format!("/v5/sessions/{}/{}", self.target_type, session_id)),
                Some(headers),
            )
            .await
            .map(|_| ())
    }

    pub async fn get_stream_state(
        &self,
        session_id: &str,
    ) -> Result<StreamStateResponse, WebApiError> {
        let headers = self.create_headers()?;
        let response = self
            .transport
            .get(
                &self.endpoint(&format!(
                    "/v5/sessions/{}/{}/state",
                    self.target_type, session_id
                )),
                Some(headers),
            )
            .await?;

        let state = response
            .get("state")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let error_details = response.get("errorDetails").map(|v| StreamErrorDetails {
            code: v.get("code").cloned(),
            message: v
                .get("message")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string()),
        });

        Ok(StreamStateResponse {
            state,
            error_details,
        })
    }

    /// 获取会话进入可协商阶段后由服务端下发的串流配置。
    ///
    /// 配置内容会随服务端版本演进，调用层保留为 JSON，避免在控制面复制一套
    /// 与 libwebrtc 绑定的配置模型。
    pub async fn get_configuration(&self, session_id: &str) -> Result<Value, WebApiError> {
        let headers = self.create_headers()?;
        self.transport
            .get(
                &self.endpoint(&format!(
                    "/v5/sessions/{}/{}/configuration",
                    self.target_type, session_id
                )),
                Some(headers),
            )
            .await
    }

    pub async fn send_connect_token(
        &self,
        session_id: &str,
        user_token: &str,
    ) -> Result<(), WebApiError> {
        let payload = json!({ "userToken": user_token });
        let headers = self.create_headers()?;
        self.transport
            .post(
                &self.endpoint(&format!(
                    "/v5/sessions/{}/{}/connect",
                    self.target_type, session_id
                )),
                payload,
                Some(headers),
            )
            .await
            .map(|_| ())
    }

    pub async fn send_keepalive(&self, session_id: &str) -> Result<(), WebApiError> {
        let headers = self.create_headers()?;
        self.transport
            .post(
                &self.endpoint(&format!(
                    "/v5/sessions/{}/{}/keepalive",
                    self.target_type, session_id
                )),
                serde_json::Value::Null,
                Some(headers),
            )
            .await
            .map(|_| ())
    }

    pub async fn get_waiting_times(
        &self,
        title_id: &str,
    ) -> Result<WaitingTimesResponse, WebApiError> {
        let headers = self.create_headers()?;
        let response = self
            .transport
            .get(
                &self.endpoint(&format!("/v1/waittime/{}", title_id)),
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn get_consoles(&self) -> Result<Vec<Value>, WebApiError> {
        let headers = self.create_headers()?;

        let response = self
            .transport
            .get(&self.endpoint("/v6/servers/home?mr=50"), Some(headers))
            .await?;

        let consoles = response
            .get("results")
            .and_then(|v| v.as_array())
            .ok_or_else(|| WebApiError::parse("Missing results in response"))?;

        Ok(consoles.clone())
    }

    pub async fn input_configs(
        &self,
        xbox_title_id: &str,
    ) -> Result<InputConfigResponse, WebApiError> {
        let payload = json!({
            "titleIds": [xbox_title_id],
            "titleIdType": "xboxTitleId"
        });

        let headers = self.create_headers()?;

        let response = self
            .transport
            .post(
                &self.endpoint("/v2/titles/inputconfigs"),
                payload,
                Some(headers),
            )
            .await?;

        Ok(InputConfigResponse {
            title_id: xbox_title_id.to_string(),
            config: response,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn authorization_header(&self) -> String {
        format!("Bearer {}", self.bearer_token)
    }

    fn create_headers(&self) -> Result<HeaderMap, WebApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        for (key, value) in &self.header_overrides {
            headers.insert(
                reqwest::header::HeaderName::from_bytes(key.as_bytes())
                    .map_err(|_| WebApiError::parse("Invalid header name"))?,
                HeaderValue::from_str(value)?,
            );
        }

        Ok(headers)
    }
}
