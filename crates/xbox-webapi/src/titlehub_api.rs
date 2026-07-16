use crate::error::WebApiError;
use crate::transport::HttpTransport;
use reqwest::header::HeaderMap;
use serde_json::Value;

const TITLEHUB_BASE_URL: &str = "https://titlehub.xboxlive.com";

pub struct TitleHubApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}

impl TitleHubApi {
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

    pub async fn get_title_history(&self, xuid: &str) -> Result<Value, WebApiError> {
        let url = title_history_url(xuid)?;
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

fn title_history_url(xuid: &str) -> Result<String, WebApiError> {
    let xuid = numeric_id(xuid, "xuid")?;
    Ok(format!(
        "{TITLEHUB_BASE_URL}/users/xuid({xuid})/titles/titlehistory/decoration/achievement,image,scid"
    ))
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
    use super::title_history_url;

    #[test]
    fn builds_title_history_url() {
        assert_eq!(
            title_history_url("2533274981234567").expect("url"),
            "https://titlehub.xboxlive.com/users/xuid(2533274981234567)/titles/titlehistory/decoration/achievement,image,scid"
        );
    }

    #[test]
    fn rejects_non_numeric_xuid() {
        assert!(title_history_url("123/path").is_err());
    }
}
