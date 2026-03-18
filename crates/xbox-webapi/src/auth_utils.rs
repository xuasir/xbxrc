use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use openssl::ec::{EcGroup, EcKey};
use openssl::nid::Nid;
use openssl::pkey::PKey;
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
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)
        .map_err(|e: openssl::error::ErrorStack| e.to_string())?;
    let key = EcKey::generate(&group).map_err(|e: openssl::error::ErrorStack| e.to_string())?;
    let pkey = PKey::from_ec_key(key).map_err(|e: openssl::error::ErrorStack| e.to_string())?;

    let ec_key = pkey
        .ec_key()
        .map_err(|e: openssl::error::ErrorStack| e.to_string())?;
    let public_key = ec_key.public_key();

    let mut ctx =
        openssl::bn::BigNumContext::new().map_err(|e: openssl::error::ErrorStack| e.to_string())?;
    let pub_bytes = public_key
        .to_bytes(
            &group,
            openssl::ec::PointConversionForm::UNCOMPRESSED,
            &mut ctx,
        )
        .map_err(|e: openssl::error::ErrorStack| e.to_string())?;

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
