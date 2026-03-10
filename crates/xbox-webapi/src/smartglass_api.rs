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
