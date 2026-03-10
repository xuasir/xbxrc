use crate::crypto::XboxSignature;
use crate::error::WebApiError;
use crate::transport::HttpTransport;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use p256::ecdsa::SigningKey;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const APP_CONFIG_APP_ID: &str = "000000004c20a908";
const APP_CONFIG_TITLE_ID: &str = "328178078";
const APP_CONFIG_REDIRECT_URI: &str = "ms-xal-000000004c20a908://auth";
const CLOUD_TRANSFER_SCOPE: &str =
    "service::http://Passport.NET/purpose::PURPOSE_XBOX_CLOUD_CONSOLE_TRANSFER_TOKEN";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct DeviceTokenResponse {
    pub Token: String,
    pub IssueInstant: String,
    pub NotAfter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SisuAuthResponse {
    #[serde(rename = "MsaOauthRedirect")]
    pub msa_oauth_redirect: String,
    #[serde(rename = "PollingUri")]
    pub polling_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
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
pub struct SisuAuthorizeResponse {
    #[serde(rename = "DeviceToken")]
    pub device_token: Option<String>,
    #[serde(rename = "TitleToken")]
    pub title_token: Option<TokenDetails<TitleClaims>>,
    #[serde(rename = "UserToken")]
    pub user_token: Option<TokenDetails<UserClaims>>,
    #[serde(rename = "AuthorizationToken")]
    pub authorization_token: Option<TokenDetails<AuthClaims>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct TokenDetails<T> {
    #[serde(rename = "IssueInstant")]
    pub issue_instant: String,
    #[serde(rename = "NotAfter")]
    pub not_after: String,
    pub Token: String,
    #[serde(rename = "DisplayClaims")]
    pub display_claims: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleClaims {
    #[serde(rename = "xti")]
    pub xti: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserClaims {
    #[serde(rename = "xui")]
    pub xui: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims {
    #[serde(rename = "xui")]
    pub xui: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct XstsTokenResponse {
    pub Token: String,
    #[serde(rename = "DisplayClaims")]
    pub display_claims: Value,
    #[serde(rename = "NotAfter")]
    pub not_after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingTokenResponse {
    #[serde(rename = "_objectCreateTime")]
    pub object_create_time: i64,
    pub data: Value,
}

pub type UserTokenResponse = OAuthTokenResponse;
pub type TitleTokenResponse = TokenDetails<TitleClaims>;

pub struct AuthApi {
    transport: HttpTransport,
}

impl AuthApi {
    pub fn new() -> Self {
        Self {
            transport: HttpTransport::new(),
        }
    }

    pub fn with_transport(transport: HttpTransport) -> Self {
        Self { transport }
    }

    pub async fn get_device_token(
        &self,
        _title_id: &str,
        device_uuid: &str,
        serial_number: &str,
        device_version: &str,
        private_jwk: &Value,
    ) -> Result<DeviceTokenResponse, WebApiError> {
        let payload = serde_json::json!({
            "Properties": {
                "AuthMethod": "ProofOfPossession",
                "DeviceType": "Android",
                "Id": format!("{{{}}}", device_uuid),
                "SerialNumber": format!("{{{}}}", serial_number),
                "ProofKey": {
                    "alg": "ES256",
                    "crv": "P-256",
                    "kty": "EC",
                    "use": "sig",
                    "x": private_jwk.get("x").and_then(|v| v.as_str()).ok_or_else(|| WebApiError::auth("Missing x in JWK"))?,
                    "y": private_jwk.get("y").and_then(|v| v.as_str()).ok_or_else(|| WebApiError::auth("Missing y in JWK"))?
                },
                "Version": device_version
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });

        let payload_str = serde_json::to_string(&payload)?;
        let d_b64 = private_jwk
            .get("d")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WebApiError::auth("Missing d in JWK"))?;
        let d_bytes = URL_SAFE_NO_PAD
            .decode(d_b64)
            .map_err(|e| WebApiError::auth(format!("Failed to decode private key: {}", e)))?;
        let signing_key = SigningKey::from_slice(&d_bytes)
            .map_err(|e| WebApiError::auth(format!("Failed to create signing key: {}", e)))?;

        let signature =
            XboxSignature::sign_request("/device/authenticate", "", &payload_str, &signing_key)
                .map_err(|e| WebApiError::auth(format!("Failed to sign request: {}", e)))?;

        let headers = HttpTransport::create_header_map(&[
            ("x-xbl-contract-version", "1"),
            ("Signature", &signature),
        ])?;

        let response = self
            .transport
            .post(
                "https://device.auth.xboxlive.com/device/authenticate",
                payload,
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn sisu_authenticate(
        &self,
        device_token: &str,
        code_challenge: &str,
        code_challenge_method: &str,
        state: &str,
        private_jwk: &Value,
    ) -> Result<SisuAuthResponse, WebApiError> {
        let payload = serde_json::json!({
            "AppId": APP_CONFIG_APP_ID,
            "TitleId": APP_CONFIG_TITLE_ID,
            "RedirectUri": APP_CONFIG_REDIRECT_URI,
            "DeviceToken": device_token,
            "Sandbox": "RETAIL",
            "TokenType": "code",
            "Offers": ["service::user.auth.xboxlive.com::MBI_SSL"],
            "Query": {
                "display": "android_phone",
                "code_challenge": code_challenge,
                "code_challenge_method": code_challenge_method,
                "state": state
            }
        });

        let payload_str = serde_json::to_string(&payload)?;
        let d_b64 = private_jwk
            .get("d")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WebApiError::auth("Missing d in JWK"))?;
        let d_bytes = URL_SAFE_NO_PAD
            .decode(d_b64)
            .map_err(|e| WebApiError::auth(format!("Failed to decode private key: {}", e)))?;
        let signing_key = SigningKey::from_slice(&d_bytes)
            .map_err(|e| WebApiError::auth(format!("Failed to create signing key: {}", e)))?;

        let signature =
            XboxSignature::sign_request("/authenticate", "", &payload_str, &signing_key)
                .map_err(|e| WebApiError::auth(format!("Failed to sign request: {}", e)))?;

        let headers = HttpTransport::create_header_map(&[
            ("x-xbl-contract-version", "1"),
            ("Signature", &signature),
        ])?;

        let response = self
            .transport
            .post(
                "https://sisu.xboxlive.com/authenticate",
                payload,
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn exchange_code_for_token(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<OAuthTokenResponse, WebApiError> {
        let body = reqwest::Url::parse_with_params(
            "https://login.live.com/oauth20_token.srf",
            &[
                ("client_id", APP_CONFIG_APP_ID),
                ("code", code),
                ("code_verifier", code_verifier),
                ("grant_type", "authorization_code"),
                ("redirect_uri", APP_CONFIG_REDIRECT_URI),
                ("scope", "service::user.auth.xboxlive.com::MBI_SSL"),
            ],
        )
        .map_err(|e| WebApiError::parse(e.to_string()))?
        .query()
        .unwrap_or_default()
        .to_string();

        let headers = HttpTransport::create_header_map(&[(
            "Content-Type",
            "application/x-www-form-urlencoded",
        )])?;

        let response = self
            .transport
            .post(
                "https://login.live.com/oauth20_token.srf",
                Value::String(body),
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn refresh_user_token(
        &self,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse, WebApiError> {
        let body = reqwest::Url::parse_with_params(
            "https://login.live.com/oauth20_token.srf",
            &[
                ("client_id", APP_CONFIG_APP_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("scope", "service::user.auth.xboxlive.com::MBI_SSL"),
            ],
        )
        .map_err(|e| WebApiError::parse(e.to_string()))?
        .query()
        .unwrap_or_default()
        .to_string();

        let headers = HttpTransport::create_header_map(&[(
            "Content-Type",
            "application/x-www-form-urlencoded",
        )])?;

        let response = self
            .transport
            .post(
                "https://login.live.com/oauth20_token.srf",
                Value::String(body),
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn get_cloud_transfer_token(
        &self,
        refresh_token: &str,
    ) -> Result<String, WebApiError> {
        let body = reqwest::Url::parse_with_params(
            "https://login.live.com/oauth20_token.srf",
            &[
                ("client_id", APP_CONFIG_APP_ID),
                ("scope", CLOUD_TRANSFER_SCOPE),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("code", ""),
                ("code_verifier", ""),
                ("redirect_uri", ""),
            ],
        )
        .map_err(|e| WebApiError::parse(e.to_string()))?
        .query()
        .unwrap_or_default()
        .to_string();

        let headers = HttpTransport::create_header_map(&[(
            "Content-Type",
            "application/x-www-form-urlencoded",
        )])?;

        let response = self
            .transport
            .post(
                "https://login.live.com/oauth20_token.srf",
                Value::String(body),
                Some(headers),
            )
            .await?;

        response
            .get("access_token")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                response
                    .get("lpt")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .ok_or_else(|| WebApiError::parse("Cloud transfer token response is invalid."))
    }

    pub async fn sisu_authorize(
        &self,
        user_token: &str,
        device_token: &str,
        private_jwk: &Value,
    ) -> Result<SisuAuthorizeResponse, WebApiError> {
        let payload = serde_json::json!({
            "AccessToken": format!("t={}", user_token),
            "AppId": APP_CONFIG_APP_ID,
            "DeviceToken": device_token,
            "Sandbox": "RETAIL",
            "SiteName": "user.auth.xboxlive.com",
            "UseModernGamertag": true,
            "ProofKey": {
                "use": "sig",
                "alg": "ES256",
                "kty": "EC",
                "crv": "P-256",
                "x": private_jwk.get("x").and_then(|v| v.as_str()).ok_or_else(|| WebApiError::auth("Missing x in JWK"))?,
                "y": private_jwk.get("y").and_then(|v| v.as_str()).ok_or_else(|| WebApiError::auth("Missing y in JWK"))?
            }
        });

        let payload_str = serde_json::to_string(&payload)?;
        let d_b64 = private_jwk
            .get("d")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WebApiError::auth("Missing d in JWK"))?;
        let d_bytes = URL_SAFE_NO_PAD
            .decode(d_b64)
            .map_err(|e| WebApiError::auth(format!("Failed to decode private key: {}", e)))?;
        let signing_key = SigningKey::from_slice(&d_bytes)
            .map_err(|e| WebApiError::auth(format!("Failed to create signing key: {}", e)))?;

        let signature = XboxSignature::sign_request("/authorize", "", &payload_str, &signing_key)
            .map_err(|e| WebApiError::auth(format!("Failed to sign request: {}", e)))?;

        let headers = HttpTransport::create_header_map(&[
            ("x-xbl-contract-version", "1"),
            ("Signature", &signature),
        ])?;

        let response = self
            .transport
            .post(
                "https://sisu.xboxlive.com/authorize",
                payload,
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn get_title_token(
        &self,
        access_token: &str,
        device_token: &str,
    ) -> Result<TokenDetails<TitleClaims>, WebApiError> {
        let payload = serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "DeviceToken": device_token,
                "RpsTicket": format!("d={}", access_token),
                "SiteName": "user.auth.xboxlive.com"
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });

        let headers = HttpTransport::create_header_map(&[("x-xbl-contract-version", "1")])?;

        let response = self
            .transport
            .post(
                "https://title.auth.xboxlive.com/title/authenticate",
                payload,
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn get_user_token(
        &self,
        access_token: &str,
    ) -> Result<TokenDetails<UserClaims>, WebApiError> {
        let payload = serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "RpsTicket": format!("d={}", access_token),
                "SiteName": "user.auth.xboxlive.com"
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });

        let headers = HttpTransport::create_header_map(&[("x-xbl-contract-version", "1")])?;

        let response = self
            .transport
            .post(
                "https://user.auth.xboxlive.com/user/authenticate",
                payload,
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn xsts_authorize(
        &self,
        user_token: &str,
        relying_party: &str,
        private_jwk: &Value,
    ) -> Result<XstsTokenResponse, WebApiError> {
        let payload = serde_json::json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [user_token]
            },
            "RelyingParty": relying_party,
            "TokenType": "JWT"
        });

        let payload_str = serde_json::to_string(&payload)?;
        let d_b64 = private_jwk
            .get("d")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WebApiError::auth("Missing d in JWK"))?;
        let d_bytes = URL_SAFE_NO_PAD
            .decode(d_b64)
            .map_err(|e| WebApiError::auth(format!("Failed to decode private key: {}", e)))?;
        let signing_key = SigningKey::from_slice(&d_bytes)
            .map_err(|e| WebApiError::auth(format!("Failed to create signing key: {}", e)))?;

        let signature =
            XboxSignature::sign_request("/xsts/authorize", "", &payload_str, &signing_key)
                .map_err(|e| WebApiError::auth(format!("Failed to sign request: {}", e)))?;

        let headers = HttpTransport::create_header_map(&[
            ("x-xbl-contract-version", "1"),
            ("Signature", &signature),
        ])?;

        let response = self
            .transport
            .post(
                "https://xsts.auth.xboxlive.com/xsts/authorize",
                payload,
                Some(headers),
            )
            .await?;

        serde_json::from_value(response).map_err(|e| WebApiError::parse(e.to_string()))
    }

    pub async fn get_streaming_token(
        &self,
        xsts_token: &str,
        offering: &str,
        force_region_ip: &str,
    ) -> Result<StreamingTokenResponse, WebApiError> {
        let payload = serde_json::json!({
            "token": xsts_token,
            "offeringId": offering
        });

        let mut headers = HttpTransport::create_header_map(&[
            ("Content-Type", "application/json"),
            ("Cache-Control", "no-store, must-revalidate, no-cache"),
            ("x-gssv-client", "XboxComBrowser"),
        ])?;

        if !force_region_ip.trim().is_empty() {
            headers.insert(
                "x-cosmos-ip",
                HeaderValue::from_str(force_region_ip.trim())
                    .map_err(|e| WebApiError::parse(e.to_string()))?,
            );
        }

        let payload_str = serde_json::to_string(&payload)?;
        headers.insert(
            "Content-Length",
            HeaderValue::from_str(&payload_str.len().to_string())
                .map_err(|e| WebApiError::parse(e.to_string()))?,
        );

        let url = format!(
            "https://{}.gssv-play-prod.xboxlive.com/v2/login/user",
            offering
        );
        let response = self.transport.post(&url, payload, Some(headers)).await?;

        Ok(StreamingTokenResponse {
            object_create_time: chrono::Utc::now().timestamp_millis(),
            data: response,
        })
    }
}

impl Default for AuthApi {
    fn default() -> Self {
        Self::new()
    }
}
