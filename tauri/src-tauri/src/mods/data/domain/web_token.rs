use serde_json::Value;

#[derive(Debug, Clone)]
pub struct WebTokenClaims {
    pub user_token: String,
    pub uhs: String,
}

// 从 webToken 快照中提取 xbox-webapi 初始化所需声明。
pub fn resolve_web_token_claims(raw: &Value) -> Option<WebTokenClaims> {
    let token = raw.get("data").unwrap_or(raw);

    let user_token = token.get("Token").and_then(|value| value.as_str())?;
    let uhs = token
        .get("DisplayClaims")
        .and_then(|value| value.get("xui"))
        .and_then(|value| value.as_array())
        .and_then(|xui| xui.first())
        .and_then(|value| value.get("uhs"))
        .and_then(|value| value.as_str())?;

    if user_token.is_empty() || uhs.is_empty() {
        return None;
    }

    Some(WebTokenClaims {
        user_token: user_token.to_string(),
        uhs: uhs.to_string(),
    })
}
