use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, AUTHORIZATION, CONTENT_TYPE,
};
use reqwest::Client;
use serde_json::Value;

pub struct XboxWebApi {
    uhs: String,
    token: String,
    client: Client,
}

impl XboxWebApi {
    pub fn new(uhs: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            uhs: uhs.into(),
            token: token.into(),
            client: Client::new(),
        }
    }

    pub fn profile(&self) -> ProfileProvider<'_> {
        ProfileProvider { api: self }
    }

    pub fn smartglass(&self) -> SmartglassProvider<'_> {
        SmartglassProvider { api: self }
    }

    fn authorization_header(&self) -> String {
        format!("XBL3.0 x={};{}", self.uhs, self.token)
    }

    async fn get_json(
        &self,
        host: &str,
        path: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<Value, String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header()).map_err(|e| e.to_string())?,
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        for (key, value) in extra_headers {
            let header_name = HeaderName::from_bytes(key.as_bytes()).map_err(|e| e.to_string())?;
            headers.insert(
                header_name,
                HeaderValue::from_str(value).map_err(|e| e.to_string())?,
            );
        }

        let url = format!("https://{}{}", host, path);
        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = response.status();
        let text = response.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "XboxWebApi request failed {} {}: {}",
                host, path, text
            ));
        }

        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}

pub struct ProfileProvider<'a> {
    api: &'a XboxWebApi,
}

impl<'a> ProfileProvider<'a> {
    pub async fn get_current_user(&self) -> Result<Value, String> {
        self.api
            .get_json(
                "profile.xboxlive.com",
                "/users/me/profile/settings?settings=GameDisplayName,GameDisplayPicRaw,Gamerscore,Gamertag",
                &[("x-xbl-contract-version", "3")],
            )
            .await
    }
}

pub struct SmartglassProvider<'a> {
    api: &'a XboxWebApi,
}

impl<'a> SmartglassProvider<'a> {
    pub async fn get_consoles_list(&self) -> Result<Value, String> {
        self.api
            .get_json(
                "xccs.xboxlive.com",
                "/lists/devices?queryCurrentDevice=false&includeStorageDevices=true",
                &[
                    ("x-xbl-contract-version", "4"),
                    ("skillplatform", "RemoteManagement"),
                ],
            )
            .await
    }
}
