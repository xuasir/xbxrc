use crate::error::WebApiError;
use crate::transport::HttpTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileResponse {
    pub profile_users: Vec<ProfileSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileApiResponse {
    pub profile_users: Vec<ProfileSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub id: String,
    pub settings: Vec<ProfileSetting>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSetting {
    pub id: String,
    pub value: Value,
}

pub struct ProfileApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}

impl ProfileApi {
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

    pub async fn get_current_user(&self) -> Result<Value, WebApiError> {
        let headers = HttpTransport::create_header_map(&[
            ("Authorization", &self.authorization_header()),
            ("Accept-Language", "en-US"),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-xbl-contract-version", "3"),
        ])?;

        let response = self
            .transport
            .get(
                "https://profile.xboxlive.com/users/me/profile/settings?settings=GameDisplayName,GameDisplayPicRaw,Gamerscore,Gamertag",
                Some(headers),
            )
            .await?;

        // 直接返回原始 JSON，让调用方处理结构
        Ok(response)
    }
}
