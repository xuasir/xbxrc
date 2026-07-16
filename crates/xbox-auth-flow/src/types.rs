use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartOAuthLoginInput {
    pub title_id: String,
    pub device_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartOAuthLoginOutput {
    pub oauth_url: String,
    pub oauth_state: String,
    pub pending: PendingOAuthLogin,
    pub seed: AuthFlowSeed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOAuthLogin {
    pub redirect_flow: xbox_webapi::XalRedirectFlow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFlowSeed {
    pub private_jwk: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteOAuthLoginInput {
    pub callback_url: String,
    pub pending: PendingOAuthLogin,
    pub seed: AuthFlowSeed,
    pub force_region_ip: String,
    pub include_streaming_tokens: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteOAuthLoginOutput {
    pub auth_bundle: AuthBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshAndFinalizeInput {
    pub refresh_token: String,
    pub seed: AuthFlowSeed,
    pub force_region_ip: String,
    pub include_streaming_tokens: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshAndFinalizeOutput {
    pub auth_bundle: AuthBundle,
    pub refreshed_user_token: xbox_webapi::OAuthTokenResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDownstreamTokensInput {
    pub user_token: xbox_webapi::OAuthTokenResponse,
    pub sisu_token: FlowSisuTokenData,
    pub seed: AuthFlowSeed,
    pub force_region_ip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDownstreamTokensOutput {
    pub auth_bundle: AuthBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTokenInput {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTokenOutput {
    pub transfer_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthBundle {
    pub user_token: xbox_webapi::OAuthTokenResponse,
    pub sisu_token: FlowSisuTokenData,
    pub web_token: serde_json::Value,
    pub stream_tokens: serde_json::Value,
    pub app_level: u32,
    pub token_update_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FlowSisuTokenData {
    pub device_token: String,
    pub title_token: FlowTokenDetails,
    pub user_token: FlowTokenDetails,
    pub authorization_token: FlowTokenDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FlowTokenDetails {
    #[serde(rename = "IssueInstant")]
    pub issue_instant: Option<String>,
    #[serde(rename = "NotAfter")]
    pub not_after: Option<String>,
    #[serde(rename = "Token")]
    pub token: Option<String>,
    #[serde(rename = "DisplayClaims")]
    pub display_claims: serde_json::Value,
}
