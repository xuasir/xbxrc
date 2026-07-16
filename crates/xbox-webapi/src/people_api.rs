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
            transport: HttpTransport::new(),
            uhs,
            token,
        }
    }

    pub async fn get_friends_count(&self) -> Result<Option<u32>, WebApiError> {
        let value = self
            .transport
            .get(
                "https://peoplehub.xboxlive.com/users/me/people/social/decoration/preferredcolor,detail,multiplayersummary,presencedetail",
                Some(self.headers()?),
            )
            .await?;
        Ok(parse_friends_count(&value))
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
}
