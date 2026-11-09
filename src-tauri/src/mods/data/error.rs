use crate::error::AppError;
use xbox_webapi::WebApiError;

/// 数据查询保留安全文案和请求错误类别，供所有页面选择恢复动作。
#[derive(Debug, Clone, thiserror::Error)]
pub enum DataError {
    #[error("{0}")]
    Message(String),
    #[error("{message}")]
    Request {
        message: String,
        source: WebApiError,
    },
}

impl DataError {
    pub(crate) fn request(source: WebApiError, message: String) -> Self {
        Self::Request { message, source }
    }

    pub(crate) fn safe_summary(&self) -> serde_json::Value {
        match self {
            Self::Message(_) => serde_json::json!({ "errorKind": "data" }),
            Self::Request { source, .. } => source.safe_summary(),
        }
    }
}

impl From<WebApiError> for DataError {
    fn from(source: WebApiError) -> Self {
        let message = super::services::personal_service::user_error(&source);
        Self::request(source, message)
    }
}

impl From<String> for DataError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for DataError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_string())
    }
}

impl From<DataError> for AppError {
    fn from(error: DataError) -> Self {
        match error {
            DataError::Message(message) => Self::Data(message),
            DataError::Request { message, source } => Self::DataDetailed {
                message,
                details: source.safe_summary(),
            },
        }
    }
}
