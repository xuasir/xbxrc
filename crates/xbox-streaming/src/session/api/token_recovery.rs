//! HTTP 执行层向既有宿主认证提供者请求一次 Bearer 恢复。
use crate::session::access::StreamingToken;
use crate::session::flow::SessionFlowProvider;
use std::sync::Arc;
use xbox_webapi::transport::{BearerRecoveryFuture, BearerTokenRecovery, HttpTransport};
use xbox_webapi::WebApiError;

struct SessionTokenRecovery<P> {
    provider: Arc<P>,
    target_type: String,
    credential_generation: Option<u64>,
}

impl<P> std::fmt::Debug for SessionTokenRecovery<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionTokenRecovery")
            .field("target", &self.target_type)
            .finish_non_exhaustive()
    }
}

impl<P: SessionFlowProvider> BearerTokenRecovery for SessionTokenRecovery<P> {
    fn recover(&self, rejected_token: &str) -> BearerRecoveryFuture {
        let provider = self.provider.clone();
        let target = self.target_type.clone();
        let rejected = rejected_token.to_string();
        let generation = self.credential_generation;
        Box::pin(async move {
            let token = provider
                .recover_streaming_token(&target, &rejected, generation)
                .await?;
            StreamingToken::parse(&token)
                .map(|token| token.gs_token)
                .map_err(|_| WebApiError::parse("invalid renewed streaming credentials"))
        })
    }
}

pub(crate) fn with_token_recovery<P: SessionFlowProvider>(
    transport: HttpTransport,
    provider: Arc<P>,
    target: &str,
    credential_generation: Option<u64>,
) -> HttpTransport {
    transport.with_bearer_recovery(Arc::new(SessionTokenRecovery {
        provider,
        target_type: target.to_string(),
        credential_generation,
    }))
}
