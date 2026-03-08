use crate::mods::data::client::XboxWebApiClient;
use crate::mods::data::domain::web_token::resolve_web_token_claims;
use crate::mods::data::domain::DataSessionContext;
use std::sync::Arc;

pub struct XboxWebApiProvider {
    current_client: Option<Arc<XboxWebApiClient>>,
    current_fingerprint: String,
}

impl XboxWebApiProvider {
    pub fn new() -> Self {
        Self {
            current_client: None,
            current_fingerprint: String::new(),
        }
    }

    // 按 token 指纹复用 client，避免重复初始化 HTTP 资源。
    pub fn get_or_create(&mut self, session: &DataSessionContext) -> Option<Arc<XboxWebApiClient>> {
        let claims = resolve_web_token_claims(&session.web_token)?;
        let fingerprint = format!("{}:{}", claims.uhs, claims.user_token);

        if let Some(client) = &self.current_client {
            if self.current_fingerprint == fingerprint {
                return Some(client.clone());
            }
        }

        let next_client = Arc::new(XboxWebApiClient::new(claims.uhs, claims.user_token));
        self.current_fingerprint = fingerprint;
        self.current_client = Some(next_client.clone());
        Some(next_client)
    }
}
