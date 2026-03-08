use crate::mods::streaming::http_client::{StreamingHttpClient, StreamingHttpError};
use crate::mods::streaming::types::{StreamingErrorDetails, StreamingQueueDetails};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct StreamingSessionApi {
    target_type: String,
    http_client: StreamingHttpClient,
    preferred_game_language: String,
    resolution: i64,
}

impl StreamingSessionApi {
    pub fn new(
        target_type: String,
        http_client: StreamingHttpClient,
        preferred_game_language: String,
        resolution: i64,
    ) -> Self {
        Self {
            target_type,
            http_client,
            preferred_game_language,
            resolution,
        }
    }

    pub async fn start_stream(&self, target_id: &str) -> Result<String, StreamingHttpError> {
        let os_name = resolve_os_name(self.resolution);
        let device_info = create_device_info(os_name);

        let payload = json!({
            "titleId": if self.target_type == "cloud" { target_id } else { "" },
            "systemUpdateGroup": "",
            "clientSessionId": "",
            "settings": {
                "nanoVersion": "V3;WebrtcTransport.dll",
                "enableTextToSpeech": false,
                "highContrast": 0,
                "locale": self.preferred_game_language,
                "useIceConnection": false,
                "timezoneOffsetMinutes": 120,
                "sdkType": "web",
                "osName": os_name
            },
            "serverId": if self.target_type == "home" { target_id } else { "" },
            "fallbackRegionNames": []
        });

        let value = self
            .http_client
            .request_json(
                "POST",
                &format!("/v5/sessions/{}/play", self.target_type),
                Some(payload),
                &[("X-MS-Device-Info", device_info)],
            )
            .await?;

        let session_path = value
            .get("sessionPath")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        Ok(session_path)
    }

    pub async fn stop_stream(&self, session_id: &str) -> Result<(), StreamingHttpError> {
        self.http_client
            .request_json(
                "DELETE",
                &format!("/v5/sessions/{}/{}", self.target_type, session_id),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    pub async fn get_stream_state(
        &self,
        session_id: &str,
    ) -> Result<(Option<String>, Option<StreamingErrorDetails>), StreamingHttpError> {
        let value = self
            .http_client
            .request_json(
                "GET",
                &format!("/v5/sessions/{}/{}/state", self.target_type, session_id),
                None,
                &[],
            )
            .await?;

        let state = value
            .get("state")
            .and_then(Value::as_str)
            .map(|text| text.to_string());

        let error_details = value.get("errorDetails").and_then(|details| {
            Some(StreamingErrorDetails {
                code: details.get("code").cloned(),
                message: details
                    .get("message")
                    .and_then(Value::as_str)
                    .map(|text| text.to_string()),
            })
        });

        Ok((state, error_details))
    }

    pub async fn send_connect_token(
        &self,
        session_id: &str,
        user_token: &str,
    ) -> Result<(), StreamingHttpError> {
        self.http_client
            .request_json(
                "POST",
                &format!("/v5/sessions/{}/{}/connect", self.target_type, session_id),
                Some(json!({ "userToken": user_token })),
                &[],
            )
            .await
            .map(|_| ())
    }

    pub async fn send_keepalive(&self, session_id: &str) -> Result<(), StreamingHttpError> {
        self.http_client
            .request_json(
                "POST",
                &format!("/v5/sessions/{}/{}/keepalive", self.target_type, session_id),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    pub async fn get_waiting_times(
        &self,
        title_id: &str,
    ) -> Result<StreamingQueueDetails, StreamingHttpError> {
        let value = self
            .http_client
            .request_json("GET", &format!("/v1/waittime/{title_id}"), None, &[])
            .await?;

        Ok(serde_json::from_value::<StreamingQueueDetails>(value).unwrap_or_default())
    }

    pub async fn get_consoles(&self) -> Result<Vec<Value>, StreamingHttpError> {
        // home 控制台列表查询沿用固定 windows device info，与 Electron 保持一致。
        let value = self
            .http_client
            .request_json(
                "GET",
                "/v6/servers/home?mr=50",
                None,
                &[("X-MS-Device-Info", create_device_info("windows"))],
            )
            .await?;

        let consoles = value
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(consoles)
    }

    pub async fn input_configs(&self, xbox_title_id: &str) -> Result<Value, StreamingHttpError> {
        // 标题输入配置查询复用当前分辨率推导的 osName，与串流主链路保持同源配置。
        let os_name = resolve_os_name(self.resolution);
        let device_info = create_device_info(os_name);
        self.http_client
            .request_json(
                "POST",
                "/v2/titles/inputconfigs",
                Some(json!({
                    "titleIds": [xbox_title_id],
                    "titleIdType": "xboxTitleId"
                })),
                &[("X-MS-Device-Info", device_info)],
            )
            .await
    }
}

fn resolve_os_name(resolution: i64) -> &'static str {
    if resolution == 1081 {
        return "tizen";
    }
    if resolution == 1080 {
        return "windows";
    }
    "android"
}

fn create_device_info(os_name: &str) -> String {
    json!({
        "appInfo": {
            "env": {
                "clientAppId": "www.xbox.com",
                "clientAppType": "browser",
                "clientAppVersion": "26.1.97",
                "clientSdkVersion": "10.3.7",
                "httpEnvironment": "prod",
                "sdkInstallId": ""
            }
        },
        "dev": {
            "hw": {
                "make": "Microsoft",
                "model": "unknown",
                "sdktype": "web"
            },
            "os": {
                "name": os_name,
                "ver": "22631.2715",
                "platform": "desktop"
            },
            "displayInfo": {
                "dimensions": {
                    "widthInPixels": 1920,
                    "heightInPixels": 1080
                },
                "pixelDensity": {
                    "dpiX": 1,
                    "dpiY": 1
                }
            },
            "browser": {
                "browserName": "chrome",
                "browserVersion": "130.0"
            }
        }
    })
    .to_string()
}
