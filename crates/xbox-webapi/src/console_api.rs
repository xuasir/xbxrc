use crate::error::WebApiError;
use crate::transport::HttpTransport;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

const XBOX_CLIENT_VERSION: &str = "39.39.22001.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleCommandResponse {
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolePowerResponse {
    pub console_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleTextResponse {
    pub console_id: String,
    pub accepted: bool,
}

pub struct ConsoleApi {
    transport: HttpTransport,
    uhs: String,
    user_token: String,
}

impl ConsoleApi {
    pub fn new(uhs: String, user_token: String) -> Self {
        Self {
            transport: HttpTransport::new(),
            uhs,
            user_token,
        }
    }

    pub fn with_transport(transport: HttpTransport, uhs: String, user_token: String) -> Self {
        Self {
            transport,
            uhs,
            user_token,
        }
    }

    fn create_headers(&self) -> Result<HeaderMap, WebApiError> {
        HttpTransport::create_header_map(&[
            (
                "Authorization",
                &format!("XBL3.0 x={};{}", self.uhs, self.user_token),
            ),
            ("Accept-Language", "en-US"),
            ("skillplatform", "RemoteManagement"),
            ("x-xbl-contract-version", "4"),
            ("x-xbl-client-name", "XboxApp"),
            ("x-xbl-client-type", "UWA"),
            ("x-xbl-client-version", XBOX_CLIENT_VERSION),
            ("Content-Type", "application/json"),
        ])
    }

    pub async fn send_command(
        &self,
        console_id: &str,
        command_type: &str,
        command: &str,
        parameters: Option<serde_json::Value>,
    ) -> Result<ConsoleCommandResponse, WebApiError> {
        let headers = self.create_headers()?;

        let body = json!({
            "destination": "Xbox",
            "type": command_type,
            "command": command,
            "sessionId": Uuid::new_v4().to_string(),
            "sourceId": "com.microsoft.smartglass",
            "parameters": parameters.unwrap_or_else(|| json!([])),
            "linkedXboxId": console_id
        });

        let _response = self
            .transport
            .post("https://xccs.xboxlive.com/commands", body, Some(headers))
            .await?;

        Ok(ConsoleCommandResponse { accepted: true })
    }

    pub async fn power_on(&self, console_id: &str) -> Result<ConsolePowerResponse, WebApiError> {
        self.send_command(console_id, "Power", "WakeUp", None)
            .await
            .map(|_| ConsolePowerResponse {
                console_id: console_id.to_string(),
                accepted: true,
            })
    }

    pub async fn power_off(&self, console_id: &str) -> Result<ConsolePowerResponse, WebApiError> {
        self.send_command(console_id, "Power", "TurnOff", None)
            .await
            .map(|_| ConsolePowerResponse {
                console_id: console_id.to_string(),
                accepted: true,
            })
    }

    pub async fn send_text(
        &self,
        console_id: &str,
        text: &str,
    ) -> Result<ConsoleTextResponse, WebApiError> {
        let parameters = json!([{
            "replacementString": text
        }]);

        self.send_command(console_id, "Shell", "InjectString", Some(parameters))
            .await
            .map(|_| ConsoleTextResponse {
                console_id: console_id.to_string(),
                accepted: true,
            })
    }
}
