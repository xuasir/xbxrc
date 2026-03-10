use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Data error: {0}")]
    Data(String),

    #[error("Streaming error: {0}")]
    Streaming(String),

    #[error("Gamepad error: {0}")]
    Gamepad(String),

    #[error("XbxEngine error: {0}")]
    XbxEngine(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("Network error: {0}")]
    Network(String),

    #[error("WebApi error: {0}")]
    WebApi(#[from] xbox_webapi::WebApiError),
}

impl AppError {
    pub fn code(&self) -> &str {
        match self {
            AppError::Internal(_) => "APP_INTERNAL",
            AppError::InvalidParams(_) => "APP_INVALID_PARAMS",
            AppError::Json(_) => "APP_JSON_ERROR",
            AppError::Auth(_) => "AUTH_ERROR",
            AppError::Config(_) => "CONFIG_ERROR",
            AppError::Data(_) => "DATA_ERROR",
            AppError::Streaming(_) => "STREAMING_ERROR",
            AppError::Gamepad(_) => "GAMEPAD_ERROR",
            AppError::XbxEngine(_) => "XBXENGINE_ERROR",
            AppError::Io(_) => "IO_ERROR",
            AppError::Tauri(_) => "TAURI_ERROR",
            AppError::Network(_) => "NETWORK_ERROR",
            AppError::WebApi(_) => "WEBAPI_ERROR",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Internal(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Internal(s.to_string())
    }
}
