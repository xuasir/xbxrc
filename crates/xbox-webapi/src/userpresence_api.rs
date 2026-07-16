use crate::{HttpTransport, WebApiError};
use reqwest::header::HeaderMap;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserPresence {
    pub state: Option<String>,
    pub device: Option<String>,
    pub current_title_name: Option<String>,
    pub rich_presence: Option<String>,
}
pub struct UserPresenceApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}
impl UserPresenceApi {
    pub fn new(uhs: String, token: String) -> Self {
        Self {
            transport: HttpTransport::new(),
            uhs,
            token,
        }
    }
    pub async fn get_current_user(&self) -> Result<UserPresence, WebApiError> {
        let value = self
            .transport
            .get(
                "https://userpresence.xboxlive.com/users/me?level=all",
                Some(self.headers()?),
            )
            .await?;
        Ok(parse_user_presence(&value))
    }
    fn headers(&self) -> Result<HeaderMap, WebApiError> {
        HttpTransport::create_header_map(&[
            (
                "Authorization",
                &format!("XBL3.0 x={};{}", self.uhs, self.token),
            ),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-xbl-contract-version", "3"),
        ])
    }
}
fn parse_user_presence(value: &Value) -> UserPresence {
    let root = value.get("data").unwrap_or(value);
    let selected = root
        .get("devices")
        .and_then(Value::as_array)
        .and_then(|devices| {
            devices.iter().find_map(|device| {
                let titles = device.get("titles")?.as_array()?;
                let title = titles
                    .iter()
                    .find(|v| v.get("state").and_then(Value::as_str) == Some("Active"))
                    .or_else(|| titles.first())?;
                Some((device, title))
            })
        });
    UserPresence {
        state: field(root, "state"),
        device: selected.and_then(|(d, _)| field(d, "type")),
        current_title_name: selected.and_then(|(_, t)| field(t, "name")),
        rich_presence: selected
            .and_then(|(_, t)| t.get("activity"))
            .and_then(|a| field(a, "richPresence")),
    }
}
fn field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn parses_active_title() {
        let v = parse_user_presence(
            &json!({"state":"Online","devices":[{"type":"XboxOne","titles":[{"name":"Home","state":"Background"},{"name":"Halo","state":"Active","activity":{"richPresence":"In Menus"}}]}]}),
        );
        assert_eq!(v.state.as_deref(), Some("Online"));
        assert_eq!(v.device.as_deref(), Some("XboxOne"));
        assert_eq!(v.current_title_name.as_deref(), Some("Halo"));
        assert_eq!(v.rich_presence.as_deref(), Some("In Menus"));
    }
}
