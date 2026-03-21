use serde_json::Value;

use crate::policy::types::TurnServer;

const HOME_FALLBACK_TURN_SERVER_URL: &str = "https://xstreaming-support.pages.dev/server.json";

/// Fallback TURN 拉取与内存缓存。
#[derive(Debug, Clone)]
pub struct FallbackTurnProvider {
    cached_turn_server: Option<Option<TurnServer>>,
    client: reqwest::Client,
}

impl Default for FallbackTurnProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FallbackTurnProvider {
    pub fn new() -> Self {
        Self {
            cached_turn_server: None,
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_by_target_type(
        &mut self,
        _target_type: &str,
    ) -> Result<Option<TurnServer>, String> {
        if let Some(value) = &self.cached_turn_server {
            return Ok(value.clone());
        }

        let value = self.fetch_fallback_turn_server().await;
        self.cached_turn_server = Some(value.clone());
        Ok(value)
    }

    async fn fetch_fallback_turn_server(&self) -> Option<TurnServer> {
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

        let value = response.json::<Value>().await.ok()?;
        parse_turn_server_value(&value)
    }
}

fn parse_turn_server_value(value: &Value) -> Option<TurnServer> {
    let url = value.get("url").and_then(Value::as_str)?.trim();
    let username = value.get("username").and_then(Value::as_str)?.trim();
    let credential = value.get("credential").and_then(Value::as_str)?.trim();

    if url.is_empty() || username.is_empty() || credential.is_empty() {
        return None;
    }

    Some(TurnServer {
        url: url.to_string(),
        username: username.to_string(),
        credential: credential.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_turn_server_value;

    #[test]
    fn parses_turn_server_value() {
        let value = json!({
            "url": "turn:example.com",
            "username": "u",
            "credential": "c"
        });

        let parsed = parse_turn_server_value(&value).unwrap();
        assert_eq!(parsed.url, "turn:example.com");
        assert_eq!(parsed.username, "u");
        assert_eq!(parsed.credential, "c");
    }

    #[test]
    fn rejects_invalid_turn_server_value() {
        let value = json!({
            "url": "",
            "username": "u",
            "credential": "c"
        });

        assert!(parse_turn_server_value(&value).is_none());
    }
}
