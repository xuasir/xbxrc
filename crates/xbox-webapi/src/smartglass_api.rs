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
            transport: HttpTransport::new(),
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
        if console_id.trim().is_empty() {
            return Err(WebApiError::parse("missing console identity"));
        }
        let mut url = reqwest::Url::parse(&format!("https://xccs.xboxlive.com/{path}"))
            .map_err(|e| WebApiError::parse(e.to_string()))?;
        if query {
            url.query_pairs_mut().append_pair("deviceId", console_id);
        } else {
            url.path_segments_mut()
                .map_err(|_| WebApiError::parse("invalid console URL"))?
                .push(console_id);
        }
        let headers = HttpTransport::create_header_map(&[
            ("Authorization", &self.authorization_header()),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-xbl-contract-version", "4"),
            ("skillplatform", "RemoteManagement"),
        ])?;
        self.transport.get(url.as_str(), Some(headers)).await
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

        Ok(response)
    }
}
