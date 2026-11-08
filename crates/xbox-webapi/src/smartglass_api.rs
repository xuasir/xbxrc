use crate::error::WebApiError;
use crate::transport::HttpTransport;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub is_active: bool,
}

pub struct SmartglassApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}

impl SmartglassApi {
    pub fn new(uhs: String, token: String) -> Self {
        Self {
            transport: HttpTransport::for_queries(),
            uhs,
            token,
        }
    }

    pub fn with_transport(transport: HttpTransport, uhs: String, token: String) -> Self {
        Self {
            transport,
            uhs,
            token,
        }
    }

    fn authorization_header(&self) -> String {
        format!("XBL3.0 x={};{}", self.uhs, self.token)
    }

    pub async fn get_installed_apps(
        &self,
        console_id: &str,
    ) -> Result<serde_json::Value, WebApiError> {
        self.get_console_query("lists/installedApps", console_id, true)
            .await
    }
    pub async fn get_storage_devices(
        &self,
        console_id: &str,
    ) -> Result<serde_json::Value, WebApiError> {
        self.get_console_query("lists/storageDevices", console_id, true)
            .await
    }
    pub async fn get_console_status(
        &self,
        console_id: &str,
    ) -> Result<serde_json::Value, WebApiError> {
        self.get_console_query("consoles", console_id, false).await
    }
    async fn get_console_query(
        &self,
        path: &str,
        console_id: &str,
        query: bool,
    ) -> Result<serde_json::Value, WebApiError> {
        let url = console_query_url(path, console_id, query)?;
        let headers = HttpTransport::create_header_map(&[
            ("Authorization", &self.authorization_header()),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-xbl-contract-version", "4"),
            ("skillplatform", "RemoteManagement"),
        ])?;
        let response = self.transport.get(url.as_str(), Some(headers)).await?;
        validate_remote_management_response(&response)?;
        Ok(response)
    }

    pub async fn get_consoles_list(&self) -> Result<serde_json::Value, WebApiError> {
        let headers = HttpTransport::create_header_map(&[
            ("Authorization", &self.authorization_header()),
            ("Accept-Language", "en-US"),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-xbl-contract-version", "4"),
            ("skillplatform", "RemoteManagement"),
        ])?;

        let response = self
            .transport
            .get(
                "https://xccs.xboxlive.com/lists/devices?queryCurrentDevice=false&includeStorageDevices=true",
                Some(headers),
            )
            .await?;

        validate_remote_management_response(&response)?;
        Ok(response)
    }
}

fn console_query_url(
    path: &str,
    console_id: &str,
    query: bool,
) -> Result<reqwest::Url, WebApiError> {
    let console_id = console_id.trim();
    if console_id.is_empty() {
        return Err(WebApiError::parse("missing console identity"));
    }
    let mut url = reqwest::Url::parse(&format!("https://xccs.xboxlive.com/{path}"))
        .map_err(|_| WebApiError::parse("invalid console URL"))?;
    if query {
        url.query_pairs_mut().append_pair("deviceId", console_id);
    } else {
        url.path_segments_mut()
            .map_err(|_| WebApiError::parse("invalid console URL"))?
            .push(console_id);
    }
    Ok(url)
}

pub(crate) fn validate_remote_management_response(
    response: &serde_json::Value,
) -> Result<(), WebApiError> {
    let root = response.get("data").unwrap_or(response);
    let Some(status) = root.get("status") else {
        return Ok(());
    };
    let code = status
        .get("errorCode")
        .and_then(|value| match value {
            serde_json::Value::String(value) => Some(value.trim().to_string()),
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .ok_or_else(|| WebApiError::parse("invalid remote management status"))?;
    // XCCS 的实际成功响应使用 OK，历史版本也返回 0 / Success。
    if code == "0" || code.eq_ignore_ascii_case("success") || code.eq_ignore_ascii_case("ok") {
        return Ok(());
    }
    let service_code = crate::error::sanitize_service_code(&code);
    let http_status = code
        .parse::<u16>()
        .ok()
        .filter(|status| (400..=599).contains(status))
        .or_else(|| match service_code.as_deref() {
            Some(
                "Unauthorized"
                | "InvalidToken"
                | "TokenExpired"
                | "ExpiredToken"
                | "InvalidAuthenticationToken",
            ) => Some(401),
            Some(
                "Forbidden"
                | "OfferingAccessDenied"
                | "UnsupportedCountry"
                | "UnsupportedRegion"
                | "SubscriptionRequired",
            ) => Some(403),
            Some("NotFound") => Some(404),
            Some("TooManyRequests") => Some(429),
            Some("ServiceUnavailable") => Some(503),
            _ => None,
        });
    match (http_status, service_code) {
        (Some(status), Some(code)) => Err(WebApiError::http_with_code(
            status,
            code,
            "remote management rejected",
        )),
        (Some(status), None) => Err(WebApiError::http(status, "remote management rejected")),
        _ => Err(WebApiError::parse("主机远程管理查询暂不可用")),
    }
}

#[cfg(test)]
#[path = "smartglass_api.test.rs"]
mod tests;
