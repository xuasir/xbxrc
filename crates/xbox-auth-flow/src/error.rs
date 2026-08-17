use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthFlowError {
    #[error("Missing private JWK in auth seed")]
    MissingPrivateJwk,

    #[error("Invalid callback URL: {0}")]
    InvalidCallbackUrl(String),

    #[error("OAuth callback contains error")]
    CallbackContainsError,

    #[error("Missing code in OAuth callback")]
    MissingCallbackCode,

    #[error("Missing state in OAuth callback")]
    MissingCallbackState,

    #[error("State mismatch in OAuth callback")]
    StateMismatch,

    #[error("SISU response missing {0}")]
    MissingSisuField(&'static str),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error(transparent)]
    Upstream(#[from] xbox_webapi::WebApiError),
}

impl AuthFlowError {
    pub fn is_retriable(&self) -> bool {
        matches!(self, Self::Upstream(error) if error.is_retriable())
    }
}

#[cfg(test)]
mod tests {
    use super::AuthFlowError;
    use xbox_webapi::WebApiError;

    #[test]
    fn classifies_only_retriable_upstream_failures() {
        assert!(AuthFlowError::from(WebApiError::network("reset")).is_retriable());
        assert!(AuthFlowError::from(WebApiError::http(503, "unavailable")).is_retriable());
        assert!(!AuthFlowError::from(WebApiError::http(401, "expired")).is_retriable());
        assert!(!AuthFlowError::Protocol("invalid response".into()).is_retriable());
    }
}
