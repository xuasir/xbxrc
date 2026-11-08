use crate::{HttpTransport, WebApiError};
use reqwest::header::HeaderMap;
use serde_json::Value;

pub struct PeopleApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}
impl PeopleApi {
    pub fn new(uhs: String, token: String) -> Self {
        Self {
            transport: HttpTransport::for_queries(),
            uhs,
            token,
        }
    }

    pub async fn get_friends_count(&self) -> Result<Option<u32>, WebApiError> {
        let value = self.get_friends().await?;
        Ok(parse_friends_count(&value))
    }

    pub async fn get_friends(&self) -> Result<Value, WebApiError> {
        self.transport
            .get(
                "https://peoplehub.xboxlive.com/users/me/people/social/decoration/preferredcolor,detail,multiplayersummary,presencedetail",
                Some(self.headers()?),
            )
            .await
    }

    fn headers(&self) -> Result<HeaderMap, WebApiError> {
        HttpTransport::create_header_map(&[
            (
                "Authorization",
                &format!("XBL3.0 x={};{}", self.uhs, self.token),
            ),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            // PeopleHub 明确要求合同版本，当前服务版本为 7。
            ("x-xbl-contract-version", "7"),
            ("Accept-Language", "en-US"),
        ])
    }
}

fn parse_friends_count(value: &Value) -> Option<u32> {
    value
        .get("data")
        .unwrap_or(value)
        .get("people")?
        .as_array()?
        .len()
        .try_into()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_people() {
        assert_eq!(parse_friends_count(&json!({"people":[{},{}]})), Some(2));
    }

    #[test]
    fn friends_request_supplies_current_contract_and_language() {
        let headers = PeopleApi::new("hash".into(), "token".into())
            .headers()
            .unwrap();
        assert_eq!(headers["x-xbl-contract-version"], "7");
        assert_eq!(headers["accept-language"], "en-US");
    }
}
