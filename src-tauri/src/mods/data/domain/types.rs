use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSessionContext {
    pub provider: String,
    pub app_level: u32,
    pub streaming_tokens: Value,
    pub web_token: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataAuthState {
    pub provider: String,
    pub is_authenticating: bool,
    pub is_authenticated: bool,
    pub app_level: u32,
}
