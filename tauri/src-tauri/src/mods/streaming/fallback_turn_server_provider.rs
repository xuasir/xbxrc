use crate::mods::streaming::types::StreamingTurnServerConfig;
use reqwest::Client;

const HOME_FALLBACK_TURN_SERVER_URL: &str = "https://xstreaming-support.pages.dev/server.json";

pub struct FallbackTurnServerProvider {
    home_turn_server: Option<Option<StreamingTurnServerConfig>>,
    client: Client,
}

impl FallbackTurnServerProvider {
    pub fn new() -> Self {
        Self {
            home_turn_server: None,
            client: Client::new(),
        }
    }

    pub async fn get_by_target_type(
        &mut self,
        target_type: &str,
    ) -> Result<Option<StreamingTurnServerConfig>, String> {
        if target_type != "home" {
            return Ok(None);
        }

        if let Some(value) = &self.home_turn_server {
            return Ok(value.clone());
        }

        let value = self.fetch_home_turn_server().await;
        self.home_turn_server = Some(value.clone());
        Ok(value)
    }

    async fn fetch_home_turn_server(&self) -> Option<StreamingTurnServerConfig> {
        let response = match self
            .client
            .get(HOME_FALLBACK_TURN_SERVER_URL)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => return None,
        };

        if !response.status().is_success() {
            return None;
        }

        let value = response.json::<serde_json::Value>().await.ok()?;
        let url = value.get("url").and_then(|item| item.as_str())?.trim();
        let username = value.get("username").and_then(|item| item.as_str())?.trim();
        let credential = value
            .get("credential")
            .and_then(|item| item.as_str())?
            .trim();

        if url.is_empty() || username.is_empty() || credential.is_empty() {
            return None;
        }

        Some(StreamingTurnServerConfig {
            url: url.to_string(),
            username: username.to_string(),
            credential: credential.to_string(),
        })
    }
}
