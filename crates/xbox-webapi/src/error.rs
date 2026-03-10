use reqwest::StatusCode;
use std::fmt;

#[derive(Debug, Clone)]
pub enum WebApiError {
    Network {
        message: String,
        retriable: bool,
    },
    Http {
        status: u16,
        code: Option<String>,
        message: String,
        retriable: bool,
    },
    Parse {
        message: String,
    },
    Auth {
        message: String,
    },
}

impl WebApiError {
    pub fn is_retriable(&self) -> bool {
        match self {
            WebApiError::Network { retriable, .. } => *retriable,
            WebApiError::Http { retriable, .. } => *retriable,
            WebApiError::Parse { .. } => false,
            WebApiError::Auth { .. } => false,
        }
    }

    pub fn to_status_code(&self) -> Option<u16> {
        match self {
            WebApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn network(message: impl Into<String>) -> Self {
        WebApiError::Network {
            message: message.into(),
            retriable: true,
        }
    }

    pub fn http(status: u16, message: impl Into<String>) -> Self {
        let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let retriable = matches!(
            status_code,
            StatusCode::REQUEST_TIMEOUT
                | StatusCode::TOO_MANY_REQUESTS
                | StatusCode::INTERNAL_SERVER_ERROR
                | StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
        );

        WebApiError::Http {
            status,
            code: None,
            message: message.into(),
            retriable,
        }
    }

    pub fn http_with_code(
        status: u16,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let retriable = matches!(
            status_code,
            StatusCode::REQUEST_TIMEOUT
                | StatusCode::TOO_MANY_REQUESTS
                | StatusCode::INTERNAL_SERVER_ERROR
                | StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
        );

        WebApiError::Http {
            status,
            code: Some(code.into()),
            message: message.into(),
            retriable,
        }
    }

    pub fn parse(message: impl Into<String>) -> Self {
        WebApiError::Parse {
            message: message.into(),
        }
    }

    pub fn auth(message: impl Into<String>) -> Self {
        WebApiError::Auth {
            message: message.into(),
        }
    }
}

impl fmt::Display for WebApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebApiError::Network { message, .. } => write!(f, "Network error: {}", message),
            WebApiError::Http {
                status,
                code,
                message,
                ..
            } => {
                if let Some(code) = code {
                    write!(f, "HTTP {} [{}]: {}", status, code, message)
                } else {
                    write!(f, "HTTP {}: {}", status, message)
                }
            }
            WebApiError::Parse { message } => write!(f, "Parse error: {}", message),
            WebApiError::Auth { message } => write!(f, "Auth error: {}", message),
        }
    }
}

impl std::error::Error for WebApiError {}

impl From<reqwest::Error> for WebApiError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() || err.is_connect() {
            WebApiError::Network {
                message: err.to_string(),
                retriable: true,
            }
        } else if err.is_status() {
            let status = err.status().map(|s| s.as_u16()).unwrap_or(0);
            WebApiError::http(status, err.to_string())
        } else {
            WebApiError::Network {
                message: err.to_string(),
                retriable: false,
            }
        }
    }
}

impl From<serde_json::Error> for WebApiError {
    fn from(err: serde_json::Error) -> Self {
        WebApiError::parse(err.to_string())
    }
}

impl From<reqwest::header::InvalidHeaderValue> for WebApiError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        WebApiError::parse(format!("Invalid header value: {}", err))
    }
}

impl From<reqwest::header::InvalidHeaderName> for WebApiError {
    fn from(err: reqwest::header::InvalidHeaderName) -> Self {
        WebApiError::parse(format!("Invalid header name: {}", err))
    }
}
