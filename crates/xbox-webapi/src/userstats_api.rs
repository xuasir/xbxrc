use crate::error::WebApiError;
use crate::transport::HttpTransport;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};

const USERSTATS_BATCH_URL: &str = "https://userstats.xboxlive.com/batch";
pub const USERSTATS_MAX_TITLES_PER_REQUEST: usize = 20;

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
            transport: HttpTransport::for_queries(),
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
            .post_query(USERSTATS_BATCH_URL, body, Some(self.headers()?))
            .await
    }

    pub async fn get_minutes_played_batch(
        &self,
        xuid: &str,
        title_ids: &[String],
    ) -> Result<Value, WebApiError> {
        let body = minutes_played_batch_body(xuid, title_ids)?;
        self.transport
            .post_query(USERSTATS_BATCH_URL, body, Some(self.headers()?))
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
    let title_id = numeric_id(title_id, "title id")?;
    let mut body = minutes_played_batch_body(xuid, &[title_id.to_string()])?;
    body["groups"] = json!([{"name":"Hero", "titleId":title_id}]);
    Ok(body)
}

fn minutes_played_batch_body(xuid: &str, title_ids: &[String]) -> Result<Value, WebApiError> {
    let xuid = numeric_id(xuid, "xuid")?;
    if title_ids.is_empty() || title_ids.len() > USERSTATS_MAX_TITLES_PER_REQUEST {
        return Err(WebApiError::parse("invalid statistics batch size"));
    }
    let title_ids = title_ids
        .iter()
        .map(|id| numeric_id(id, "title id"))
        .collect::<Result<Vec<_>, _>>()?;

    // Hero 装饰在多标题查询中会触发 400；批量仅请求所需的 MinutesPlayed 统计。
    Ok(json!({
        "arrangebyfield": "xuid",
        "xuids": [xuid],
        "stats": title_ids.iter().map(|id| json!({"name": "MinutesPlayed", "titleId": id})).collect::<Vec<_>>()
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

    #[test]
    fn batches_multiple_titles_and_bounds_request_size() {
        let body = super::minutes_played_batch_body("123", &["1".into(), "2".into()]).unwrap();
        assert!(body.get("groups").is_none());
        assert_eq!(
            body["stats"],
            json!([{"name":"MinutesPlayed","titleId":"1"},{"name":"MinutesPlayed","titleId":"2"}])
        );
        assert!(super::minutes_played_batch_body("123", &[]).is_err());
        assert!(super::minutes_played_batch_body("123", &vec!["1".into(); 20]).is_ok());
        assert!(super::minutes_played_batch_body("123", &vec!["1".into(); 21]).is_err());
        assert!(super::minutes_played_batch_body("123", &["invalid".into()]).is_err());
    }
}
