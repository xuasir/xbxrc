use crate::XboxBridgeError;
use xbox_webapi::{NetworkErrorKind, WebApiError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum XboxQueryFailureKind {
    Network,
    Timeout,
    Authentication,
    AccessRestricted,
    RateLimited,
    ServiceUnavailable,
    InvalidResponse,
    Unknown,
}

/// 仅传递稳定事实；接口正文与自由文本留在公共HTTP诊断边界。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxQueryFailure {
    pub kind: XboxQueryFailureKind,
    pub status_code: Option<u16>,
    pub retriable: bool,
    pub retry_after_seconds: Option<u64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxSourceFailure {
    pub source: String,
    pub failure: XboxQueryFailure,
}

impl XboxQueryFailure {
    pub fn from_safe_summary(value: &serde_json::Value) -> Self {
        let status_code = value
            .get("statusCode")
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u16::try_from(v).ok());
        let kind = match status_code {
            Some(401) => XboxQueryFailureKind::Authentication,
            Some(403) => XboxQueryFailureKind::AccessRestricted,
            Some(429) => XboxQueryFailureKind::RateLimited,
            Some(408 | 504) => XboxQueryFailureKind::Timeout,
            Some(500..=599) => XboxQueryFailureKind::ServiceUnavailable,
            _ if value
                .get("networkErrorKind")
                .and_then(serde_json::Value::as_str)
                == Some("timeout") =>
            {
                XboxQueryFailureKind::Timeout
            }
            _ => match value.get("errorKind").and_then(serde_json::Value::as_str) {
                Some("network") => XboxQueryFailureKind::Network,
                Some("parse") => XboxQueryFailureKind::InvalidResponse,
                Some("auth") => XboxQueryFailureKind::Authentication,
                _ => XboxQueryFailureKind::Unknown,
            },
        };
        Self {
            kind,
            status_code,
            retriable: value
                .get("retriable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            retry_after_seconds: None,
        }
    }
    pub fn from_web_api(error: &WebApiError) -> Self {
        let kind = match error {
            WebApiError::Network {
                kind: NetworkErrorKind::Timeout,
                ..
            } => XboxQueryFailureKind::Timeout,
            WebApiError::Network { .. } => XboxQueryFailureKind::Network,
            WebApiError::Auth { .. } | WebApiError::Http { status: 401, .. } => {
                XboxQueryFailureKind::Authentication
            }
            WebApiError::Http { status: 403, .. } => XboxQueryFailureKind::AccessRestricted,
            WebApiError::Http { status: 429, .. } => XboxQueryFailureKind::RateLimited,
            WebApiError::Http {
                status: 408 | 504, ..
            } => XboxQueryFailureKind::Timeout,
            WebApiError::Http {
                status: 500..=599, ..
            } => XboxQueryFailureKind::ServiceUnavailable,
            WebApiError::Parse { .. } => XboxQueryFailureKind::InvalidResponse,
            _ => XboxQueryFailureKind::Unknown,
        };
        Self {
            kind,
            status_code: error.to_status_code(),
            retriable: error.is_retriable(),
            retry_after_seconds: None,
        }
    }
}

pub fn query_error(error: WebApiError) -> XboxBridgeError {
    XboxBridgeError::Query {
        failure: XboxQueryFailure::from_web_api(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifications_keep_transport_facts_and_discard_raw_payloads() {
        for (status, kind) in [
            (401, XboxQueryFailureKind::Authentication),
            (403, XboxQueryFailureKind::AccessRestricted),
            (429, XboxQueryFailureKind::RateLimited),
            (503, XboxQueryFailureKind::ServiceUnavailable),
        ] {
            let error = WebApiError::Http {
                status,
                code: None,
                message: "https://secret.example/token=private".into(),
                retriable: status >= 429,
            };
            let failure = XboxQueryFailure::from_web_api(&error);
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.status_code, Some(status));
            assert!(!format!("{failure:?}").contains("secret"));
        }
        assert_eq!(
            XboxQueryFailure::from_web_api(&WebApiError::network_kind(NetworkErrorKind::Timeout))
                .kind,
            XboxQueryFailureKind::Timeout
        );
    }
}
