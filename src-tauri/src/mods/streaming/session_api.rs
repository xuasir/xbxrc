use crate::mods::streaming::types::{
    StreamingErrorDetails, StreamingHttpError, StreamingQueueDetails,
};
use serde_json::{json, Value};
use xbox_webapi::{
    ConsoleInfo, InputConfigResponse, SessionApi as CrateSessionApi, StartStreamResponse,
    StreamStateResponse, WaitingTimesResponse, WebApiError,
};

#[derive(Clone)]
pub struct StreamingSessionApi {
    target_type: String,
    session_api: CrateSessionApi,
    preferred_game_language: String,
    resolution: i64,
}

impl StreamingSessionApi {
    pub fn new(
        target_type: String,
        session_api: CrateSessionApi,
        preferred_game_language: String,
        resolution: i64,
    ) -> Self {
        Self {
            target_type,
            session_api,
            preferred_game_language,
            resolution,
        }
    }

    pub async fn start_stream(&self, target_id: &str) -> Result<String, StreamingHttpError> {
        let os_name = resolve_os_name(self.resolution);
        let _device_info = create_device_info(os_name);

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

        let response = self
            .session_api
            .start_stream_with_payload(&payload)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })?;

        Ok(response.session_path)
    }

    pub async fn stop_stream(&self, session_id: &str) -> Result<(), StreamingHttpError> {
        self.session_api
            .stop_stream(session_id)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })
    }

    pub async fn get_stream_state(
        &self,
        session_id: &str,
    ) -> Result<(Option<String>, Option<StreamingErrorDetails>), StreamingHttpError> {
        let response = self
            .session_api
            .get_stream_state(session_id)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })?;

        let error_details = response.error_details.and_then(|details| {
            Some(StreamingErrorDetails {
                code: details.code,
                message: details.message,
            })
        });

        Ok((response.state, error_details))
    }

    pub async fn send_connect_token(
        &self,
        session_id: &str,
        user_token: &str,
    ) -> Result<(), StreamingHttpError> {
        self.session_api
            .send_connect_token(session_id, user_token)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })
    }

    pub async fn send_keepalive(&self, session_id: &str) -> Result<(), StreamingHttpError> {
        self.session_api
            .send_keepalive(session_id)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })
    }

    pub async fn get_waiting_times(
        &self,
        title_id: &str,
    ) -> Result<StreamingQueueDetails, StreamingHttpError> {
        let response = self
            .session_api
            .get_waiting_times(title_id)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })?;

        Ok(StreamingQueueDetails {
            estimated_total_wait_time_in_seconds: response.estimated_total_wait_time_in_seconds,
            estimated_allocation_time_in_seconds: response.estimated_allocation_time_in_seconds,
            estimated_provisioning_time_in_seconds: response.estimated_provisioning_time_in_seconds,
        })
    }

    pub async fn get_consoles(&self) -> Result<Vec<Value>, StreamingHttpError> {
        let consoles = self
            .session_api
            .get_consoles()
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })?;

        Ok(consoles
            .into_iter()
            .map(|console| {
                json!({
                    "id": console.id,
                    "name": console.name,
                    "deviceType": console.device_type,
                    "isActive": console.is_active
                })
            })
            .collect())
    }

    pub async fn input_configs(&self, xbox_title_id: &str) -> Result<Value, StreamingHttpError> {
        let response = self
            .session_api
            .input_configs(xbox_title_id)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })?;

        Ok(json!({
            "titleId": response.title_id,
            "config": response.config
        }))
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
