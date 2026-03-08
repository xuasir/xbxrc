use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use openssl::ec::{EcGroup, EcKey};
use openssl::nid::Nid;
use openssl::pkey::PKey;
use rand::RngExt;
use reqwest::Client;
pub use reqwest::Response;
use sha2::{Digest, Sha256};

use crate::mods::auth::crypto::XboxSignature;
use crate::mods::auth::types::JwtKeysPayload;

const APP_CONFIG_APP_ID: &str = "000000004c20a908";
const APP_CONFIG_TITLE_ID: &str = "328178078";
const APP_CONFIG_REDIRECT_URI: &str = "ms-xal-000000004c20a908://auth";

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct LoginCodeChallenge {
    pub value: String,
    pub method: String,
    pub verifier: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct XalRedirectFlow {
    pub sisu_auth: serde_json::Value,
    pub state: String,
    pub code_challenge: LoginCodeChallenge,
}

pub struct XboxWebApiClient {
    http: Client,
}

impl XboxWebApiClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
    }

    pub fn generate_ecdsa_keypair() -> Result<JwtKeysPayload, String> {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(|e| e.to_string())?;
        let key = EcKey::generate(&group).map_err(|e| e.to_string())?;
        let pkey = PKey::from_ec_key(key).map_err(|e| e.to_string())?;

        let ec_key = pkey.ec_key().map_err(|e| e.to_string())?;
        let public_key = ec_key.public_key();

        let mut ctx = openssl::bn::BigNumContext::new().map_err(|e| e.to_string())?;
        let pub_bytes = public_key
            .to_bytes(
                &group,
                openssl::ec::PointConversionForm::UNCOMPRESSED,
                &mut ctx,
            )
            .map_err(|e| e.to_string())?;

        if pub_bytes.len() != 65 || pub_bytes[0] != 0x04 {
            return Err("Invalid public key format".to_string());
        }

        let x = URL_SAFE_NO_PAD.encode(&pub_bytes[1..33]);
        let y = URL_SAFE_NO_PAD.encode(&pub_bytes[33..65]);
        let d = URL_SAFE_NO_PAD.encode(ec_key.private_key().to_vec());

        let mut jwk = serde_json::Map::new();
        jwk.insert(
            "kty".to_string(),
            serde_json::Value::String("EC".to_string()),
        );
        jwk.insert(
            "crv".to_string(),
            serde_json::Value::String("P-256".to_string()),
        );
        jwk.insert(
            "alg".to_string(),
            serde_json::Value::String("ES256".to_string()),
        );
        jwk.insert(
            "use".to_string(),
            serde_json::Value::String("sig".to_string()),
        );
        jwk.insert("x".to_string(), serde_json::Value::String(x));
        jwk.insert("y".to_string(), serde_json::Value::String(y));
        jwk.insert("d".to_string(), serde_json::Value::String(d));

        Ok(JwtKeysPayload {
            private_jwk: Some(serde_json::Value::Object(jwk)),
        })
    }

    pub fn create_code_challenge() -> LoginCodeChallenge {
        let mut verifier_bytes = [0u8; 32];
        rand::rng().fill(&mut verifier_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        let value = URL_SAFE_NO_PAD.encode(hash);

        LoginCodeChallenge {
            value,
            method: "S256".to_string(),
            verifier,
        }
    }

    pub fn get_random_state() -> String {
        let mut state_bytes = [0u8; 32];
        rand::rng().fill(&mut state_bytes);
        URL_SAFE_NO_PAD.encode(state_bytes)
    }

    pub async fn get_device_token(
        &self,
        _title_id: &str,
        device_uuid: &str,
        serial_number: &str,
        device_version: &str,
        private_jwk: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
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
                    "x": private_jwk.get("x").and_then(|v| v.as_str()).ok_or("Missing x in JWK")?,
                    "y": private_jwk.get("y").and_then(|v| v.as_str()).ok_or("Missing y in JWK")?
                },
                "Version": device_version
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });

        let payload_str = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

        let d_b64 = private_jwk
            .get("d")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing d in JWK".to_string())?;
        let d_bytes = URL_SAFE_NO_PAD.decode(d_b64).map_err(|e| e.to_string())?;
        let signing_key =
            p256::ecdsa::SigningKey::from_slice(&d_bytes).map_err(|e| e.to_string())?;

        let signature =
            XboxSignature::sign_request("/device/authenticate", "", &payload_str, &signing_key)
                .map_err(|e| e.to_string())?;

        let res = self
            .http
            .post("https://device.auth.xboxlive.com/device/authenticate")
            .header("x-xbl-contract-version", "1")
            .header("Signature", signature)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let text = res.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse device token: {:?}", e))
    }

    pub async fn do_sisu_authentication(
        &self,
        device_token: &str,
        code_challenge: &LoginCodeChallenge,
        state: &str,
        private_jwk: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
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
                "code_challenge": code_challenge.value,
                "code_challenge_method": code_challenge.method,
                "state": state
            }
        });

        let payload_str = serde_json::to_string(&payload).unwrap();
        let d_b64 = private_jwk.get("d").unwrap().as_str().unwrap();
        let d_bytes = URL_SAFE_NO_PAD.decode(d_b64).map_err(|e| e.to_string())?;
        let signing_key =
            p256::ecdsa::SigningKey::from_slice(&d_bytes).map_err(|e| e.to_string())?;

        let signature =
            XboxSignature::sign_request("/authenticate", "", &payload_str, &signing_key)
                .map_err(|e| e.to_string())?;

        let res = self
            .http
            .post("https://sisu.xboxlive.com/authenticate")
            .header("x-xbl-contract-version", "1")
            .header("Signature", signature)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let text = res.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Failed to sisu authenticate: {:?}", e))
    }

    pub async fn exchange_code_for_token(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<serde_json::Value, String> {
        let params = vec![
            ("client_id", APP_CONFIG_APP_ID),
            ("code", code),
            ("code_verifier", code_verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", APP_CONFIG_REDIRECT_URI),
            ("scope", "service::user.auth.xboxlive.com::MBI_SSL"),
        ];

        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();

        let res = self
            .http
            .post("https://login.live.com/oauth20_token.srf")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e: reqwest::Error| e.to_string())?;

        let text = res
            .text()
            .await
            .map_err(|e: reqwest::Error| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Failed to exchange code: {:?}", e))
    }

    pub async fn refresh_user_token(
        &self,
        refresh_token: &str,
    ) -> Result<serde_json::Value, String> {
        let params = vec![
            ("client_id", APP_CONFIG_APP_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", "service::user.auth.xboxlive.com::MBI_SSL"),
        ];

        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();

        let res = self
            .http
            .post("https://login.live.com/oauth20_token.srf")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e: reqwest::Error| e.to_string())?;

        let text = res
            .text()
            .await
            .map_err(|e: reqwest::Error| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Failed to refresh user token: {:?}", e))
    }

    pub async fn do_sisu_authorization(
        &self,
        user_token: &str,
        device_token: &str,
        private_jwk: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
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
                "x": private_jwk.get("x").and_then(|v| v.as_str()).ok_or_else(|| "Missing x in JWK".to_string())?,
                "y": private_jwk.get("y").and_then(|v| v.as_str()).ok_or_else(|| "Missing y in JWK".to_string())?
            }
        });

        let payload_str = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let d_b64 = private_jwk
            .get("d")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing d in JWK".to_string())?;
        let d_bytes = URL_SAFE_NO_PAD.decode(d_b64).map_err(|e| e.to_string())?;
        let signing_key =
            p256::ecdsa::SigningKey::from_slice(&d_bytes).map_err(|e| e.to_string())?;

        let signature = XboxSignature::sign_request("/authorize", "", &payload_str, &signing_key)
            .map_err(|e| e.to_string())?;

        let res = self
            .http
            .post("https://sisu.xboxlive.com/authorize")
            .header("x-xbl-contract-version", "1")
            .header("Signature", signature)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let text = res.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Failed to sisu authorize: {:?}", e))
    }

    pub async fn get_title_token(
        &self,
        access_token: &str,
        device_token: &str,
    ) -> Result<serde_json::Value, String> {
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

        let res = self
            .http
            .post("https://title.auth.xboxlive.com/title/authenticate")
            .header("x-xbl-contract-version", "1")
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let text = res.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse title token: {:?}", e))
    }

    pub async fn get_user_token(&self, access_token: &str) -> Result<serde_json::Value, String> {
        let payload = serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "RpsTicket": format!("d={}", access_token),
                "SiteName": "user.auth.xboxlive.com"
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });

        let res = self
            .http
            .post("https://user.auth.xboxlive.com/user/authenticate")
            .header("x-xbl-contract-version", "1")
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let text = res.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse user token: {:?}", e))
    }

    pub async fn do_xsts_authorization(
        &self,
        sisu_user_token: &str,
        relying_party: &str,
        private_jwk: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let payload = serde_json::json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [sisu_user_token]
            },
            "RelyingParty": relying_party,
            "TokenType": "JWT"
        });

        let payload_str = serde_json::to_string(&payload).unwrap();
        let d_b64 = private_jwk.get("d").unwrap().as_str().unwrap();
        let d_bytes = URL_SAFE_NO_PAD.decode(d_b64).map_err(|e| e.to_string())?;
        let signing_key =
            p256::ecdsa::SigningKey::from_slice(&d_bytes).map_err(|e| e.to_string())?;

        let signature =
            XboxSignature::sign_request("/xsts/authorize", "", &payload_str, &signing_key)
                .map_err(|e| e.to_string())?;

        let res = self
            .http
            .post("https://xsts.auth.xboxlive.com/xsts/authorize")
            .header("x-xbl-contract-version", "1")
            .header("Signature", signature)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = res.status();
        let text = res.text().await.map_err(|e| e.to_string())?;

        if !status.is_success() {
            return Err(format!(
                "XSTS request failed with status {}: {}",
                status, text
            ));
        }

        serde_json::from_str(&text).map_err(|e| format!("Failed to parse xsts token: {:?}", e))
    }

    pub async fn get_streaming_token(
        &self,
        xsts_token: &str,
        offering: &str,
        force_region_ip: &str,
    ) -> Result<serde_json::Value, String> {
        let payload = serde_json::json!({
            "token": xsts_token,
            "offeringId": offering
        });

        let mut request = self
            .http
            .post(format!(
                "https://{}.gssv-play-prod.xboxlive.com/v2/login/user",
                offering
            ))
            .header("Content-Type", "application/json")
            .header("Cache-Control", "no-store, must-revalidate, no-cache")
            .header("x-gssv-client", "XboxComBrowser");

        if !force_region_ip.trim().is_empty() {
            request = request.header("x-cosmos-ip", force_region_ip.trim());
        }

        let response = request
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = response.status();
        let text = response.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "stream token request failed [{}]: {}",
                status, text
            ));
        }

        let data: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("Invalid stream token: {:?}", e))?;

        Ok(serde_json::json!({
            "_objectCreateTime": chrono::Utc::now().timestamp_millis(),
            "data": data
        }))
    }
}
