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
