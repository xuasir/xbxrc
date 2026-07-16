use crate::{HttpTransport, WebApiError};
use reqwest::header::HeaderMap;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SocialSummary {
    pub followers_count: Option<u32>,
    pub following_count: Option<u32>,
}

pub struct SocialApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}

impl SocialApi {
    pub fn new(uhs: String, token: String) -> Self {
        Self {
            transport: HttpTransport::new(),
            uhs,
            token,
        }
    }
    pub async fn get_summary(&self) -> Result<SocialSummary, WebApiError> {
        let value = self
            .transport
            .get(
                "https://social.xboxlive.com/users/me/summary",
                Some(self.headers()?),
            )
            .await?;
        Ok(parse_social_summary(&value))
    }
    fn headers(&self) -> Result<HeaderMap, WebApiError> {
        HttpTransport::create_header_map(&[
            (
                "Authorization",
                &format!("XBL3.0 x={};{}", self.uhs, self.token),
            ),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
        ])
    }
}

fn parse_social_summary(value: &Value) -> SocialSummary {
    let root = value.get("data").unwrap_or(value);
    let count = |key| {
        root.get("peopleCount")
            .and_then(|v| v.get(key))
            .and_then(as_u32)
    };
    SocialSummary {
        followers_count: count("followers"),
        following_count: count("following"),
    }
}
fn as_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|v| v.try_into().ok())
        .or_else(|| value.as_str()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn parses_counts() {
        let v = parse_social_summary(&json!({"peopleCount":{"followers":12,"following":"7"}}));
        assert_eq!(v.followers_count, Some(12));
        assert_eq!(v.following_count, Some(7));
    }
}
