use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTokenData {
    pub token_type: String,
    pub expires_in: u64,
    pub scope: String,
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: Option<String>,
    pub expires_on: Option<String>,
    pub ext_expires_in: Option<u64>,
    pub id_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SisuTokenData {
    pub device_token: String,
    pub title_token: TokenDetails<TitleClaims>,
    pub user_token: TokenDetails<UserClaims>,
    pub authorization_token: TokenDetails<AuthClaims>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TokenDetails<T> {
    pub issue_instant: String,
    pub not_after: String,
    pub token: String,
    pub display_claims: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleClaims {
    pub xti: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserClaims {
    pub xui: Vec<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims {
    pub xui: Vec<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtKeysPayload {
    #[serde(rename = "privateJwk")]
    pub private_jwk: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoreTokenPayload {
    #[serde(rename = "userToken")]
    pub user_token: Option<UserTokenData>,
    #[serde(rename = "sisuToken")]
    pub sisu_token: Option<SisuTokenData>,
    #[serde(rename = "jwtKeys")]
    pub jwt_keys: Option<JwtKeysPayload>,
    #[serde(rename = "tokenUpdateTime")]
    pub token_update_time: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub provider: String,
    pub is_authenticating: bool,
    pub is_authenticated: bool,
    pub app_level: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSessionReadyEvent {
    pub provider: String,
    pub app_level: u32,
    pub streaming_tokens: serde_json::Value,
    pub web_token: serde_json::Value,
}
