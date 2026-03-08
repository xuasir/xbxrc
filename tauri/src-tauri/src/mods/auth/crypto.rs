use crate::error::AppResult;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct XboxSignature;

impl XboxSignature {
    pub fn get_windows_timestamp() -> AppResult<u64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "System time went backwards")?
            .as_secs();

        Ok((now + 11_644_473_600) * 10_000_000)
    }

    pub fn sign_request(
        url_path: &str,
        auth_token: &str,
        payload: &str,
        signing_key: &SigningKey,
    ) -> AppResult<String> {
        let windows_timestamp = Self::get_windows_timestamp()?;
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

        Ok(STANDARD.encode(header_buffer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;

    #[test]
    fn test_get_windows_timestamp() {
        let ts = XboxSignature::get_windows_timestamp();
        assert!(ts.is_ok());
        let val = ts.unwrap();
        // 验证时间戳在合理范围内（2024年之后）
        assert!(val > 133_000_000_000_000_000);
    }

    #[test]
    fn test_sign_request_format() {
        let key_bytes = [1u8; 32];
        let signing_key = SigningKey::from_slice(&key_bytes).unwrap();
        let sig = XboxSignature::sign_request("/test", "token", "{}", &signing_key);
        assert!(sig.is_ok());
        let sig_str = sig.unwrap();
        // Base64 编码后的二进制头部固定至少包含 12 字节 (Version+Timestamp) + 64 字节 (Signature)
        // 约 76 字节编码后长度 > 100
        assert!(sig_str.len() > 100);
    }
}
