use crate::mods::data::application::services::{
    HostService, ProfileService, StreamingQueryService, XcloudService,
};
use crate::mods::data::domain::DataSessionContext;
use crate::mods::data::infrastructure::bridges::AuthServiceBridge;
use crate::mods::data::infrastructure::XboxWebApiProvider;
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostSummary, DataSendTextResult, DataStreamingTitleInputConfig,
    DataUserProfile, DataXcloudTitleSummary,
};
use tauri::AppHandle;

pub struct DataService {
    auth_bridge: AuthServiceBridge,
    web_api_provider: XboxWebApiProvider,
    host_service: HostService,
    profile_service: ProfileService,
    streaming_query_service: StreamingQueryService,
    xcloud_service: XcloudService,
}

impl DataService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            auth_bridge: AuthServiceBridge::new(app_handle.clone()),
            web_api_provider: XboxWebApiProvider::new(),
            host_service: HostService::new(),
            profile_service: ProfileService::new(app_handle.clone()),
            streaming_query_service: StreamingQueryService::new(app_handle.clone()),
            xcloud_service: XcloudService::new(app_handle),
        }
    }

    pub async fn get_user_profile(&mut self) -> Result<DataUserProfile, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            self.profile_service.clear_cached_profile()?;
            return self.profile_service.get_cached_profile(0);
        };

        if let Some(web_api) = self.resolve_web_api_client(&session) {
            let _ = self
                .profile_service
                .refresh_profile(&session, &web_api)
                .await;
        }

        self.profile_service.get_cached_profile(session.app_level)
    }

    pub async fn get_hosts(&mut self) -> Result<Vec<DataHostSummary>, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            eprintln!("[data][hosts] skip: no authenticated session");
            return Ok(Vec::new());
        };

        let Some(web_api) = self.resolve_web_api_client(&session) else {
            eprintln!("[data][hosts] skip: web api client unavailable");
            return Ok(Vec::new());
        };

        let hosts = self.host_service.get_hosts(&session, &web_api).await?;
        eprintln!("[data][hosts] service result count={}", hosts.len());
        Ok(hosts)
    }

    pub async fn get_remote_consoles(&mut self) -> Result<Vec<DataHostSummary>, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(Vec::new());
        };

        self.streaming_query_service
            .get_remote_consoles(&session)
            .await
    }

    pub async fn get_streaming_title_input_config(
        &mut self,
        xbox_title_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(DataStreamingTitleInputConfig {
                xbox_title_id: xbox_title_id.to_string(),
                config: serde_json::json!({}),
            });
        };

        self.streaming_query_service
            .get_streaming_title_input_config(&session, xbox_title_id)
            .await
    }

    pub async fn power_on_console(
        &mut self,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(DataConsolePowerResult {
                console_id: console_id.to_string(),
                accepted: false,
            });
        };

        self.streaming_query_service
            .power_on_console(&session, console_id)
            .await
    }

    pub async fn power_off_console(
        &mut self,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(DataConsolePowerResult {
                console_id: console_id.to_string(),
                accepted: false,
            });
        };

        self.streaming_query_service
            .power_off_console(&session, console_id)
            .await
    }

    pub async fn send_text_to_console(
        &mut self,
        console_id: &str,
        text: &str,
    ) -> Result<DataSendTextResult, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(DataSendTextResult {
                console_id: console_id.to_string(),
                accepted: false,
            });
        };

        self.streaming_query_service
            .send_text_to_console(&session, console_id, text)
            .await
    }

    pub async fn get_xcloud_titles(&mut self) -> Result<Vec<DataXcloudTitleSummary>, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(Vec::new());
        };

        self.xcloud_service.get_titles(&session).await
    }

    async fn ensure_authenticated_session(&self) -> Result<Option<DataSessionContext>, String> {
        if let Some(session) = self.auth_bridge.get_active_session().await? {
            return Ok(Some(session));
        }

        self.auth_bridge.check_authentication().await?;
        self.auth_bridge.get_active_session().await
    }

    fn resolve_web_api_client(
        &mut self,
        session: &DataSessionContext,
    ) -> Option<std::sync::Arc<crate::mods::data::client::XboxWebApiClient>> {
        self.web_api_provider.get_or_create(session)
    }
}
