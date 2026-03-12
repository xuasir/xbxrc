use serde_json::{json, Value};
use xbox_webapi::{SessionApi as CrateSessionApi, WebApiError};

use crate::policy::Plan;
use crate::session::access::StreamingToken;
use crate::session::monitor::{QueueDetails, SessionErrorDetails};

/// 基于 xbox-webapi 的会话网关：封装 payload 细节与返回值适配。
#[derive(Clone)]
pub struct WebApiSessionGateway {
    plan: Plan,
    session_api: CrateSessionApi,
}

impl WebApiSessionGateway {
    /// RFC: 凭证属于执行层，结合 Plan 策略构建 Gateway。
    /// 执行层不再自行推导设备画像，统一消费编译产出的 headers。
    pub fn new(plan: Plan, token: StreamingToken) -> Self {
        let target_type = plan.session.target.as_str();

        let session_api = CrateSessionApi::new(
            target_type.to_string(),
            plan.session.base_url.clone(),
            token.gs_token,
            plan.session.headers.clone(),
        );

        Self { plan, session_api }
    }

    pub async fn start_stream(&self) -> Result<String, WebApiError> {
        let target_id = &self.plan.session.target_id;
        let is_cloud = !self.plan.session.target.is_home();
        let settings = &self.plan.session.settings;

        // RFC: 策略只能由 policy 编译。执行层仅搬运 plan 决策值。
        let payload = json!({
            "titleId": if is_cloud { target_id } else { "" },
            "systemUpdateGroup": "",
            "clientSessionId": "",
            "settings": {
                "nanoVersion": settings.nano_version,
                "enableTextToSpeech": settings.enable_text_to_speech,
                "highContrast": settings.high_contrast,
                "locale": settings.locale,
                "useIceConnection": settings.use_ice_connection,
                "timezoneOffsetMinutes": settings.timezone_offset_minutes,
                "sdkType": settings.sdk_type,
                "osName": settings.os_name
            },
            "serverId": if !is_cloud { target_id } else { "" },
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
    ) -> Result<(Option<String>, Option<SessionErrorDetails>), WebApiError> {
        let response = self.session_api.get_stream_state(session_id).await?;
        let error_details = response.error_details.map(|details| SessionErrorDetails {
            code: details.code,
            message: details.message,
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

    pub async fn get_waiting_times(&self) -> Result<QueueDetails, WebApiError> {
        let response = self
            .session_api
            .get_waiting_times(&self.plan.session.target_id)
            .await?;

        Ok(QueueDetails {
            estimated_total_wait_time_in_seconds: response.estimated_total_wait_time_in_seconds,
            estimated_allocation_time_in_seconds: response.estimated_allocation_time_in_seconds,
            estimated_provisioning_time_in_seconds: response.estimated_provisioning_time_in_seconds,
        })
    }

    pub async fn get_remote_consoles(&self) -> Result<Vec<Value>, WebApiError> {
        self.session_api.get_consoles().await
    }

    pub async fn input_configs(&self, xbox_title_id: &str) -> Result<Value, WebApiError> {
        let response = self.session_api.input_configs(xbox_title_id).await?;
        Ok(to_legacy_input_config(response.config))
    }
}

fn to_legacy_input_config(config: Value) -> Value {
    config
}
