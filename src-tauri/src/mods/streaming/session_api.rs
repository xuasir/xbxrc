use crate::mods::streaming::types::{StreamingErrorDetails, StreamingQueueDetails};
use serde_json::{json, Value};
use xbox_webapi::{SessionApi as CrateSessionApi, WebApiError};

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

    pub async fn start_stream(&self, target_id: &str) -> Result<String, WebApiError> {
        let os_name = resolve_os_name(self.resolution);

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

        let response = self.session_api.start_stream_with_payload(&payload).await?;

        Ok(response.session_path)
    }

    pub async fn stop_stream(&self, session_id: &str) -> Result<(), WebApiError> {
        self.session_api.stop_stream(session_id).await
    }

    pub async fn get_stream_state(
        &self,
        session_id: &str,
    ) -> Result<(Option<String>, Option<StreamingErrorDetails>), WebApiError> {
        let response = self.session_api.get_stream_state(session_id).await?;

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
    ) -> Result<(), WebApiError> {
        self.session_api
            .send_connect_token(session_id, user_token)
            .await
    }

    pub async fn send_keepalive(&self, session_id: &str) -> Result<(), WebApiError> {
        self.session_api.send_keepalive(session_id).await
    }

    pub async fn get_waiting_times(
        &self,
        title_id: &str,
    ) -> Result<StreamingQueueDetails, WebApiError> {
        let response = self.session_api.get_waiting_times(title_id).await?;

        Ok(StreamingQueueDetails {
            estimated_total_wait_time_in_seconds: response.estimated_total_wait_time_in_seconds,
            estimated_allocation_time_in_seconds: response.estimated_allocation_time_in_seconds,
            estimated_provisioning_time_in_seconds: response.estimated_provisioning_time_in_seconds,
        })
    }

    pub async fn get_consoles(&self) -> Result<Vec<Value>, WebApiError> {
        let consoles = self.session_api.get_consoles().await?;

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

    pub async fn input_configs(&self, xbox_title_id: &str) -> Result<Value, WebApiError> {
        let response = self.session_api.input_configs(xbox_title_id).await?;

        Ok(to_legacy_input_config(response.config))
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

fn to_legacy_input_config(config: Value) -> Value {
    config
}

#[cfg(test)]
mod tests {
    use super::to_legacy_input_config;
    use serde_json::json;

    #[test]
    fn keeps_input_config_payload_in_legacy_shape() {
        let payload = json!({
            "inputConfigs": [
                {
                    "titleId": "12345",
                    "supportsTouch": true
                }
            ]
        });

        assert_eq!(to_legacy_input_config(payload.clone()), payload);
    }
}
