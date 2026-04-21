use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::rand_core::OsRng;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtKeysPayload {
    pub private_jwk: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginCodeChallenge {
    pub value: String,
    pub method: String,
    pub verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XalRedirectFlow {
    pub sisu_auth: serde_json::Value,
    pub state: String,
    pub code_challenge: LoginCodeChallenge,
}

pub fn generate_ecdsa_keypair() -> Result<JwtKeysPayload, String> {
    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let pub_bytes = verifying_key.to_encoded_point(false);
    let pub_bytes = pub_bytes.as_bytes();

    if pub_bytes.len() != 65 || pub_bytes[0] != 0x04 {
        return Err("Invalid public key format".to_string());
    }

    let x = URL_SAFE_NO_PAD.encode(&pub_bytes[1..33]);
    let y = URL_SAFE_NO_PAD.encode(&pub_bytes[33..65]);
    let d = URL_SAFE_NO_PAD.encode(signing_key.to_bytes());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_ecdsa_keypair_produces_valid_p256_jwk_shape() {
        let payload = generate_ecdsa_keypair().expect("generate keypair");
        let jwk = payload.private_jwk.expect("private_jwk");
        let obj = jwk.as_object().expect("jwk object");
        assert_eq!(obj.get("kty").and_then(|v| v.as_str()), Some("EC"));
        assert_eq!(obj.get("crv").and_then(|v| v.as_str()), Some("P-256"));
        assert_eq!(obj.get("alg").and_then(|v| v.as_str()), Some("ES256"));
        assert_eq!(obj.get("use").and_then(|v| v.as_str()), Some("sig"));
        assert!(obj.get("x").and_then(|v| v.as_str()).is_some());
        assert!(obj.get("y").and_then(|v| v.as_str()).is_some());
        assert!(obj.get("d").and_then(|v| v.as_str()).is_some());
    }
}
