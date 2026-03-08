use reqwest::Client;
use url::form_urlencoded::Serializer;

use crate::mods::auth::repository::CoreTokenRepository;

const XAL_APP_ID: &str = "000000004c20a908";
const CLOUD_TRANSFER_SCOPE: &str =
    "service::http://Passport.NET/purpose::PURPOSE_XBOX_CLOUD_CONSOLE_TRANSFER_TOKEN";

pub struct AuthTransferTokenService {
    core_repository: CoreTokenRepository,
    http_client: Client,
}

impl AuthTransferTokenService {
    pub fn new(core_repository: CoreTokenRepository) -> Self {
        Self {
            core_repository,
            http_client: Client::new(),
        }
    }

    pub async fn get_transfer_token(&self) -> Result<String, String> {
        let user_token = self
            .core_repository
            .get_user_token()?
            .ok_or("Refresh token is missing. Please authenticate first.")?;

        let refresh_token = user_token.refresh_token.trim().to_string();

        if refresh_token.is_empty() {
            return Err("Refresh token is missing. Please authenticate first.".to_string());
        }

        let payload = Serializer::new(String::new())
            .append_pair("client_id", XAL_APP_ID)
            .append_pair("scope", CLOUD_TRANSFER_SCOPE)
            .append_pair("grant_type", "refresh_token")
            .append_pair("refresh_token", refresh_token.as_str())
            .append_pair("code", "")
            .append_pair("code_verifier", "")
            .append_pair("redirect_uri", "")
            .finish();

        let response = self
            .http_client
            .post("https://login.live.com/oauth20_token.srf")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = response.status();
        let text = response.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "Transfer token request failed [{}]: {}",
                status, text
            ));
        }

        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

        if let Some(token) = value.get("access_token").and_then(|v| v.as_str()) {
            if !token.is_empty() {
                return Ok(token.to_string());
            }
        }

        if let Some(token) = value.get("lpt").and_then(|v| v.as_str()) {
            if !token.is_empty() {
                return Ok(token.to_string());
            }
        }

        Err("Cloud transfer token response is invalid.".to_string())
    }
}
