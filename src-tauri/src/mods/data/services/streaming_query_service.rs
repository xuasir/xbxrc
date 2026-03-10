use crate::mods::config::ConfigProviderRef;
use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostSummary, DataSendTextResult, DataSessionContext,
    DataStreamingTitleInputConfig,
};
use crate::mods::streaming::api_provider::StreamingApiProvider;
use crate::mods::streaming::session_api::StreamingSessionApi;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use uuid::Uuid;

const XBOX_CLIENT_VERSION: &str = "39.39.22001.0";

pub struct StreamingQueryService {
    client: Client,
    streaming_api_provider: StreamingApiProvider,
}

impl StreamingQueryService {
    pub fn new(config_provider: ConfigProviderRef) -> Self {
        Self {
            client: Client::new(),
            streaming_api_provider: StreamingApiProvider::new(config_provider),
        }
    }

    pub async fn get_remote_consoles(
        &self,
        session: &DataSessionContext,
    ) -> Result<Vec<DataHostSummary>, String> {
        let Some(api) = self.create_home_session_api(session).await? else {
            return Ok(Vec::new());
        };

        let consoles = api
            .get_consoles()
            .await
            .map_err(|error| error.to_string())?;

        let mut summaries = Vec::new();
        for console in consoles {
            if let Ok(summary) = serde_json::from_value::<DataHostSummary>(console) {
                summaries.push(summary);
            }
        }

        Ok(summaries)
    }

    pub async fn get_streaming_title_input_config(
        &self,
        session: &DataSessionContext,
        xbox_title_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, String> {
        let Some(api) = self.create_home_session_api(session).await? else {
            return Ok(DataStreamingTitleInputConfig {
                xbox_title_id: xbox_title_id.to_string(),
                config: json!({}),
            });
        };

        let config = api
            .input_configs(xbox_title_id)
            .await
            .map_err(|error| error.to_string())?;

        Ok(DataStreamingTitleInputConfig {
            xbox_title_id: xbox_title_id.to_string(),
            config,
        })
    }

    pub async fn power_on_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, String> {
        self.send_console_power_command(session, console_id, "WakeUp")
            .await
    }

    pub async fn power_off_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, String> {
        self.send_console_power_command(session, console_id, "TurnOff")
            .await
    }

    pub async fn send_text_to_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
        text: &str,
    ) -> Result<DataSendTextResult, String> {
        let accepted = self
            .send_console_command(
                session,
                console_id,
                "Shell",
                "InjectString",
                Some(json!([
                    {
                        "replacementString": text
                    }
                ])),
            )
            .await?;

        Ok(DataSendTextResult {
            console_id: console_id.to_string(),
            accepted,
        })
    }

    async fn send_console_power_command(
        &self,
        session: &DataSessionContext,
        console_id: &str,
        command: &str,
    ) -> Result<DataConsolePowerResult, String> {
        let accepted = self
            .send_console_command(session, console_id, "Power", command, None)
            .await?;

        Ok(DataConsolePowerResult {
            console_id: console_id.to_string(),
            accepted,
        })
    }

    async fn send_console_command(
        &self,
        session: &DataSessionContext,
        console_id: &str,
        command_type: &str,
        command: &str,
        parameters: Option<Value>,
    ) -> Result<bool, String> {
        let Some(claims) = resolve_web_token_claims(&session.web_token) else {
            return Ok(false);
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("XBL3.0 x={};{}", claims.uhs, claims.user_token))
                .map_err(|error| error.to_string())?,
        );
        headers.insert("Accept-Language", HeaderValue::from_static("en-US"));
        headers.insert(
            "skillplatform",
            HeaderValue::from_static("RemoteManagement"),
        );
        headers.insert("x-xbl-contract-version", HeaderValue::from_static("4"));
        headers.insert("x-xbl-client-name", HeaderValue::from_static("XboxApp"));
        headers.insert("x-xbl-client-type", HeaderValue::from_static("UWA"));
        headers.insert(
            "x-xbl-client-version",
            HeaderValue::from_static(XBOX_CLIENT_VERSION),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let body = json!({
            "destination": "Xbox",
            "type": command_type,
            "command": command,
            "sessionId": Uuid::new_v4().to_string(),
            "sourceId": "com.microsoft.smartglass",
            "parameters": parameters.unwrap_or_else(|| json!([])),
            "linkedXboxId": console_id
        });

        let response = self
            .client
            .post("https://xccs.xboxlive.com/commands")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| error.to_string())?;

        Ok(response.status().is_success())
    }

    async fn create_home_session_api(
        &self,
        session: &DataSessionContext,
    ) -> Result<Option<StreamingSessionApi>, String> {
        // data 域对齐 Electron 语义：home 查询统一复用 streaming 基础设施。
        let Some(token) = resolve_xhome_token(session) else {
            return Ok(None);
        };
        let api = self
            .streaming_api_provider
            .create_session_api(token, "home")
            .await?;
        Ok(Some(api))
    }
}

fn resolve_xhome_token(session: &DataSessionContext) -> Option<&Value> {
    session
        .streaming_tokens
        .get("xHomeToken")
        .or_else(|| session.streaming_tokens.get("xhomeToken"))
}
