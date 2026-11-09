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

    #[error("OAuth refresh grant failed: {0}")]
    RefreshGrant(#[source] xbox_webapi::WebApiError),

    #[error(transparent)]
    Upstream(#[from] xbox_webapi::WebApiError),
}

impl AuthFlowError {
    pub fn upstream_error(&self) -> Option<&xbox_webapi::WebApiError> {
        match self {
            Self::Upstream(error) | Self::RefreshGrant(error) => Some(error),
            _ => None,
        }
    }

    pub fn requires_interactive_login(&self) -> bool {
        matches!(self, Self::RefreshGrant(error) if error.to_status_code() == Some(401)
            || error.safe_service_code().as_deref() == Some("invalid_grant"))
    }

    pub fn is_retriable(&self) -> bool {
        self.upstream_error()
            .is_some_and(|error| error.is_retriable())
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

    #[test]
    fn interactive_login_requires_a_rejected_refresh_grant() {
        for error in [
            WebApiError::http(401, "secret"),
            WebApiError::http_with_code(400, "invalid_grant", "secret"),
        ] {
            assert!(AuthFlowError::RefreshGrant(error.clone()).requires_interactive_login());
            assert!(!AuthFlowError::Upstream(error).requires_interactive_login());
        }
        for error in [
            WebApiError::network("secret"),
            WebApiError::http(403, "secret"),
            WebApiError::http(429, "secret"),
            WebApiError::http(503, "secret"),
            WebApiError::parse("secret"),
        ] {
            assert!(!AuthFlowError::RefreshGrant(error).requires_interactive_login());
        }
    }
}
