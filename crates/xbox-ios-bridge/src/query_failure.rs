use crate::XboxBridgeError;
use xbox_webapi::WebApiError;

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
    pub network_error_kind: Option<String>,
    pub service_code: Option<String>,
    pub protocol_error_code: Option<String>,
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
            network_error_kind: value["networkErrorKind"]
                .as_str()
                .filter(|kind| {
                    [
                        "dns", "connect", "tls", "timeout", "body", "request", "unknown",
                    ]
                    .contains(kind)
                })
                .map(str::to_owned),
            service_code: value["serviceCode"]
                .as_str()
                .and_then(xbox_webapi::sanitize_service_code),
            protocol_error_code: value["protocolErrorCode"]
                .as_str()
                .filter(|code| {
                    [
                        "missingTitles",
                        "missingPeople",
                        "missingAchievements",
                        "missingMedia",
                        "missingConsoleResults",
                        "invalidConsoleList",
                        "invalidConsoleServiceStatus",
                        "consoleServiceRejected",
                        "invalidConsoleStatus",
                        "invalidConsoleStorage",
                        "invalidConsoleStorageIdentity",
                        "missingConsoleStorage",
                        "invalidStatistics",
                        "invalidProfile",
                    ]
                    .contains(code)
                })
                .map(str::to_owned),
        }
    }
    pub fn from_web_api(error: &WebApiError) -> Self {
        let mut failure = Self::from_safe_summary(&error.safe_summary());
        failure.network_error_kind = error.network_error_kind().map(|kind| kind.as_str().into());
        failure
    }
}

pub fn query_error(error: WebApiError) -> XboxBridgeError {
    if matches!(&error, WebApiError::Parse { message } if message.starts_with("operationCancelled:"))
    {
        return XboxBridgeError::Cancelled;
    }
    XboxBridgeError::Query {
        failure: XboxQueryFailure::from_web_api(&error),
    }
}

/// 凭据续期继续保留固定网络分类，不回传底层错误正文。
pub(crate) fn credential_error(error: XboxBridgeError) -> WebApiError {
    use xbox_webapi::NetworkErrorKind;
    match error {
        XboxBridgeError::Query { failure } => match failure.kind {
            XboxQueryFailureKind::Network | XboxQueryFailureKind::Timeout => {
                let kind = match failure.network_error_kind.as_deref() {
                    Some("dns") => NetworkErrorKind::Dns,
                    Some("connect") => NetworkErrorKind::Connect,
                    Some("tls") => NetworkErrorKind::Tls,
                    Some("body") => NetworkErrorKind::Body,
                    Some("request") => NetworkErrorKind::Request,
                    Some("timeout") => NetworkErrorKind::Timeout,
                    _ if failure.kind == XboxQueryFailureKind::Timeout => NetworkErrorKind::Timeout,
                    _ => NetworkErrorKind::Unknown,
                };
                WebApiError::network_kind(kind)
            }
            _ if failure.status_code.is_some() => WebApiError::Http {
                status: failure.status_code.unwrap(),
                code: failure.service_code,
                message: "streaming credential renewal unavailable".into(),
                retriable: failure.retriable,
            },
            XboxQueryFailureKind::Authentication => {
                WebApiError::http(401, "streaming credential renewal rejected")
            }
            _ => WebApiError::parse("streaming credential response unavailable"),
        },
        XboxBridgeError::RetryableAuthentication(_) => {
            WebApiError::http(503, "streaming credential renewal temporarily unavailable")
        }
        XboxBridgeError::Authentication(_) => {
            WebApiError::http(401, "streaming credential renewal rejected")
        }
        XboxBridgeError::Cancelled => WebApiError::parse("operationCancelled:stage=authRenewal"),
        _ => WebApiError::parse("streaming credential response unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbox_webapi::NetworkErrorKind;
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

    #[test]
    fn preserves_all_network_subtypes_and_only_allowlisted_codes() {
        for kind in [
            NetworkErrorKind::Dns,
            NetworkErrorKind::Connect,
            NetworkErrorKind::Tls,
            NetworkErrorKind::Timeout,
            NetworkErrorKind::Body,
            NetworkErrorKind::Request,
            NetworkErrorKind::Unknown,
        ] {
            let failure = XboxQueryFailure::from_web_api(&WebApiError::network_kind(kind));
            assert_eq!(failure.network_error_kind.as_deref(), Some(kind.as_str()));
        }
        let failure =
            XboxQueryFailure::from_web_api(&WebApiError::parse("invalid console list response"));
        assert_eq!(
            failure.protocol_error_code.as_deref(),
            Some("invalidConsoleList")
        );
        let failure = XboxQueryFailure::from_web_api(&WebApiError::http_with_code(
            503,
            "ServerNeverRegistered",
            "secret",
        ));
        assert_eq!(
            failure.service_code.as_deref(),
            Some("ServerNeverRegistered")
        );
        let failure = XboxQueryFailure::from_safe_summary(&serde_json::json!({
            "networkErrorKind": "https://secret.invalid", "serviceCode": "Bearer private",
            "protocolErrorCode": "account-private", "body": "secret"
        }));
        assert_eq!(failure.network_error_kind, None);
        assert_eq!(failure.service_code, None);
        assert_eq!(failure.protocol_error_code, None);
        assert!(!format!("{failure:?}").contains("private"));
    }
}
