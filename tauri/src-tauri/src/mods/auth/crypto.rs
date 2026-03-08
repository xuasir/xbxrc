use base64::{engine::general_purpose::STANDARD, Engine as _};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct XboxSignature;

impl XboxSignature {
    pub fn get_windows_timestamp() -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        (now + 11_644_473_600) * 10_000_000
    }

    pub fn sign_request(
        url_path: &str,
        auth_token: &str,
        payload: &str,
        signing_key: &SigningKey,
    ) -> String {
        let windows_timestamp = Self::get_windows_timestamp();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&1u32.to_be_bytes());
        buffer.push(0);
        buffer.extend_from_slice(&windows_timestamp.to_be_bytes());
        buffer.push(0);
        buffer.extend_from_slice(b"POST");
        buffer.push(0);
        buffer.extend_from_slice(url_path.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(auth_token.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(payload.as_bytes());
        buffer.push(0);

        let signature: Signature = signing_key.sign(&buffer);
        let sig_bytes = signature.to_bytes();

        let mut header_buffer = Vec::new();
        header_buffer.extend_from_slice(&1u32.to_be_bytes());
        header_buffer.extend_from_slice(&windows_timestamp.to_be_bytes());
        header_buffer.extend_from_slice(&sig_bytes);

        STANDARD.encode(header_buffer)
    }
}
