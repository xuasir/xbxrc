use crate::error::WebApiError;
use crate::transport::HttpTransport;
use reqwest::header::HeaderMap;
use serde_json::Value;

const ACHIEVEMENTS_BASE_URL: &str = "https://achievements.xboxlive.com";

pub struct AchievementsApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}

impl AchievementsApi {
    pub fn new(uhs: String, token: String) -> Self {
        Self {
            transport: HttpTransport::new(),
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

    pub async fn get_title_achievements(
        &self,
        xuid: &str,
        title_id: &str,
        continuation_token: Option<&str>,
        max_items: Option<u32>,
    ) -> Result<Value, WebApiError> {
        let url = title_achievements_url(xuid, title_id, continuation_token, max_items)?;
        self.transport.get(&url, Some(self.headers()?)).await
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

fn title_achievements_url(
    xuid: &str,
    title_id: &str,
    continuation_token: Option<&str>,
    max_items: Option<u32>,
) -> Result<String, WebApiError> {
    let xuid = numeric_id(xuid, "xuid")?;
    let title_id = numeric_id(title_id, "title id")?;
    let mut url = reqwest::Url::parse(&format!(
        "{ACHIEVEMENTS_BASE_URL}/users/xuid({xuid})/achievements"
    ))
    .map_err(|error| WebApiError::parse(error.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("titleid", title_id);
        if let Some(max_items) = max_items {
            query.append_pair("maxItems", &max_items.to_string());
        }
        if let Some(continuation_token) = continuation_token {
            query.append_pair("continuationToken", continuation_token);
        }
    }
    Ok(url.to_string())
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
    use super::title_achievements_url;

    #[test]
    fn builds_title_achievements_url() {
        assert_eq!(
            title_achievements_url("2533274981234567", "1292135258", None, Some(200))
                .expect("url"),
            "https://achievements.xboxlive.com/users/xuid(2533274981234567)/achievements?titleid=1292135258&maxItems=200"
        );
    }

    #[test]
    fn rejects_non_numeric_title_id() {
        assert!(title_achievements_url("2533274981234567", "halo", None, None).is_err());
    }

    #[test]
    fn percent_encodes_continuation_token() {
        let url =
            title_achievements_url("2533274981234567", "1292135258", Some("next page/+"), None)
                .expect("url");

        assert!(url.contains("continuationToken=next+page%2F%2B"));
    }
}
