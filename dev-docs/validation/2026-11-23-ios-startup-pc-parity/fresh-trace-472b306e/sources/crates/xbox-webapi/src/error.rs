use reqwest::StatusCode;
use serde_json::{json, Value};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkErrorKind {
    Dns,
    Connect,
    Tls,
    Timeout,
    Body,
    Request,
    Unknown,
}

impl NetworkErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Connect => "connect",
            Self::Tls => "tls",
            Self::Timeout => "timeout",
            Self::Body => "body",
            Self::Request => "request",
            Self::Unknown => "unknown",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Dns => "DNS resolution failed",
            Self::Connect => "connection failed",
            Self::Tls => "TLS connection failed",
            Self::Timeout => "request timed out",
            Self::Body => "response body interrupted",
            Self::Request => "request initialization failed",
            Self::Unknown => "network request failed",
        }
    }
}

#[derive(Debug, Clone)]
pub enum WebApiError {
    Network {
        message: String,
        retriable: bool,
        kind: NetworkErrorKind,
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
            kind: NetworkErrorKind::Unknown,
        }
    }

    pub fn network_kind(kind: NetworkErrorKind) -> Self {
        Self::Network {
            message: kind.message().to_string(),
            retriable: matches!(
                kind,
                NetworkErrorKind::Dns
                    | NetworkErrorKind::Connect
                    | NetworkErrorKind::Timeout
                    | NetworkErrorKind::Body
            ),
            kind,
        }
    }

    pub fn network_error_kind(&self) -> Option<NetworkErrorKind> {
        match self {
            Self::Network { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub fn safe_service_code(&self) -> Option<String> {
        match self {
            Self::Http {
                code: Some(code), ..
            } => sanitize_service_code(code),
            _ => None,
        }
    }

    pub fn safe_summary(&self) -> Value {
        let kind = match self {
            Self::Network { .. } => "network",
            Self::Http { .. } => "http",
            Self::Parse { .. } => "parse",
            Self::Auth { .. } => "auth",
        };
        let mut summary = json!({
            "errorKind": kind,
            "statusCode": self.to_status_code(),
            "retriable": self.is_retriable(),
        });
        if let Some(kind) = self
            .network_error_kind()
            .filter(|kind| *kind != NetworkErrorKind::Unknown)
        {
            summary["networkErrorKind"] = json!(kind.as_str());
        }
        if let Some(code) = self.safe_service_code() {
            summary["serviceCode"] = json!(code);
        }
        if let Self::Parse { message } = self {
            let code = match message.as_str() {
                "missing titles array" => Some("missingTitles"),
                "missing people array" => Some("missingPeople"),
                "missing achievements array" => Some("missingAchievements"),
                "missing values array" => Some("missingMedia"),
                "missing result array" => Some("missingConsoleResults"),
                "invalid console list response" => Some("invalidConsoleList"),
                "invalid remote management status" => Some("invalidConsoleServiceStatus"),
                "主机远程管理查询暂不可用" => Some("consoleServiceRejected"),
                "invalid console status response" => Some("invalidConsoleStatus"),
                "invalid console storage response" => Some("invalidConsoleStorage"),
                "invalid console storage identity" => Some("invalidConsoleStorageIdentity"),
                "missing storageDevices array" => Some("missingConsoleStorage"),
                "invalid statistics response" => Some("invalidStatistics"),
                "invalid personal profile response"
                | "invalid profile response: empty settings" => Some("invalidProfile"),
                _ => None,
            };
            if let Some(code) = code {
                summary["protocolErrorCode"] = json!(code);
            }
        }
        summary
    }

    /// 将下载等 HTTP 路径的底层错误转换成固定分类，过滤 URL 与错误正文。
    pub fn from_reqwest_safe(error: reqwest::Error) -> Self {
        if let Some(status) = error.status() {
            return Self::http_response(status.as_u16(), "");
        }
        let error = error.without_url();
        let kind = if error.is_timeout() {
            NetworkErrorKind::Timeout
        } else {
            let mut messages = Vec::new();
            let mut source: Option<&dyn std::error::Error> = Some(&error);
            while let Some(cause) = source {
                messages.push(cause.to_string().to_ascii_lowercase());
                source = cause.source();
            }
            classify_network_sources(&messages, error.is_connect(), error.is_body())
        };
        // source 链只用于固定分类；URL、代理凭据和系统错误正文留在内存中。
        Self::network_kind(kind)
    }

    pub(crate) fn http_response(status: u16, body: &str) -> Self {
        let code = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| extract_service_code(&value, 0));
        match code {
            Some(code) => Self::http_with_code(status, code, "request rejected by service"),
            None => Self::http(status, "request rejected by service"),
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
                kind: NetworkErrorKind::Unknown,
            }
        } else if err.is_status() {
            let status = err.status().map(|s| s.as_u16()).unwrap_or(0);
            WebApiError::http(status, err.to_string())
        } else {
            WebApiError::Network {
                message: err.to_string(),
                retriable: false,
                kind: NetworkErrorKind::Unknown,
            }
        }
    }
}

pub fn sanitize_service_code(value: &str) -> Option<String> {
    const CODES: &[&str] = &[
        "BadRequest",
        "Unauthorized",
        "Forbidden",
        "NotFound",
        "TooManyRequests",
        "InternalServerError",
        "ServiceUnavailable",
        "UnsupportedCountry",
        "UnsupportedRegion",
        "BlockedRegion",
        "RegionNotSupported",
        "OfferingAccessDenied",
        "OfferingNotFound",
        "InvalidToken",
        "TokenExpired",
        "ExpiredToken",
        "InvalidAuthenticationToken",
        "AuthenticationRequired",
        "AuthenticationFailed",
        "AuthorizationFailed",
        "NotAuthorized",
        "InvalidArgument",
        "SubscriptionRequired",
        "NoValidSubscription",
        "UserNotAuthenticated",
        "invalid_grant",
        "invalid_request",
        "invalid_client",
        "unauthorized_client",
        "access_denied",
        "temporarily_unavailable",
        "server_error",
        "interaction_required",
    ];
    if let Some(code) = CODES
        .iter()
        .find(|code| code.eq_ignore_ascii_case(value.trim()))
    {
        return Some((*code).to_string());
    }
    value
        .strip_prefix("XErr:")?
        .parse::<u32>()
        .ok()
        .map(|code| format!("XErr:{code}"))
}

fn extract_service_code(value: &Value, depth: usize) -> Option<String> {
    if depth > 3 {
        return None;
    }
    if let Some(code) = value.as_str().and_then(sanitize_service_code) {
        return Some(code);
    }
    if let Some(code) = value
        .get("XErr")
        .and_then(Value::as_u64)
        .and_then(|code| u32::try_from(code).ok())
    {
        return Some(format!("XErr:{code}"));
    }
    let code = ["code", "errorCode", "Code", "ErrorCode", "error"]
        .into_iter()
        .find_map(|key| {
            value
                .get(key)
                .and_then(|value| extract_service_code(value, depth + 1))
        });
    let detail = ["message", "Message", "innererror", "innerError", "details"]
        .into_iter()
        .find_map(|key| {
            value.get(key).and_then(|value| {
                if let Some(array) = value.as_array() {
                    array
                        .iter()
                        .find_map(|value| extract_service_code(value, depth + 1))
                } else {
                    extract_service_code(value, depth + 1)
                }
            })
        });
    // 通用 HTTP 类别后面的具体准入码更适合诊断地区与账户资格。
    if matches!(
        code.as_deref(),
        None | Some("Forbidden") | Some("Unauthorized") | Some("BadRequest")
    ) {
        detail.or(code)
    } else {
        code
    }
}

fn classify_network_sources(messages: &[String], connect: bool, body: bool) -> NetworkErrorKind {
    let contains = |needles: &[&str]| {
        messages
            .iter()
            .any(|message| needles.iter().any(|needle| message.contains(needle)))
    };
    if contains(&[
        "dns",
        "resolve host",
        "resolve address",
        "lookup address",
        "name or service not known",
        "nodename nor servname",
    ]) {
        NetworkErrorKind::Dns
    } else if contains(&["tls", "ssl", "certificate", "handshake"]) {
        NetworkErrorKind::Tls
    } else if connect {
        NetworkErrorKind::Connect
    } else if body {
        NetworkErrorKind::Body
    } else {
        NetworkErrorKind::Request
    }
}

#[cfg(test)]
#[path = "error.test.rs"]
mod tests;

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
