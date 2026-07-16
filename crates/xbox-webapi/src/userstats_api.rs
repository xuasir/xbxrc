use crate::error::WebApiError;
use crate::transport::{HttpTransport, RetryMode};
use reqwest::header::HeaderMap;
use serde_json::{json, Value};

const USERSTATS_BATCH_URL: &str = "https://userstats.xboxlive.com/batch";

#[derive(Clone)]
pub struct UserStatsApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}

impl UserStatsApi {
    pub fn new(uhs: String, token: String) -> Self {
        Self {
            // /batch 是只读统计查询，允许安全重试 POST。
            transport: HttpTransport::with_retry_mode(RetryMode::Always),
            uhs,
            token,
        }
    }

    pub fn with_transport(transport: HttpTransport, uhs: String, token: String) -> Self {
        Self {
            transport,
            uhs,
            token,
        }
    }

    pub async fn get_minutes_played(
        &self,
        xuid: &str,
        title_id: &str,
    ) -> Result<Value, WebApiError> {
        let body = minutes_played_body(xuid, title_id)?;
        self.transport
            .post(USERSTATS_BATCH_URL, body, Some(self.headers()?))
            .await
    }

    fn headers(&self) -> Result<HeaderMap, WebApiError> {
        HttpTransport::create_header_map(&[
            ("Authorization", &self.authorization_header()),
            ("Accept-Language", "en-US"),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-xbl-contract-version", "2"),
        ])
    }

    fn authorization_header(&self) -> String {
        format!("XBL3.0 x={};{}", self.uhs, self.token)
    }
}

fn minutes_played_body(xuid: &str, title_id: &str) -> Result<Value, WebApiError> {
    let xuid = numeric_id(xuid, "xuid")?;
    let title_id = numeric_id(title_id, "title id")?;

    Ok(json!({
        "arrangebyfield": "xuid",
        "xuids": [xuid],
        "groups": [{"name": "Hero", "titleId": title_id}],
        "stats": [{"name": "MinutesPlayed", "titleId": title_id}]
    }))
}

fn numeric_id<'a>(value: &'a str, label: &str) -> Result<&'a str, WebApiError> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WebApiError::parse(format!("invalid {label}")));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::minutes_played_body;
    use serde_json::json;

    #[test]
    fn builds_single_title_minutes_played_body() {
        assert_eq!(
            minutes_played_body("2533274981234567", "1292135258").expect("body"),
            json!({
                "arrangebyfield": "xuid",
                "xuids": ["2533274981234567"],
                "groups": [{"name": "Hero", "titleId": "1292135258"}],
                "stats": [{"name": "MinutesPlayed", "titleId": "1292135258"}]
            })
        );
    }

    #[test]
    fn rejects_non_numeric_ids() {
        assert!(minutes_played_body("me", "1292135258").is_err());
        assert!(minutes_played_body("2533274981234567", "game").is_err());
    }
}
