use crate::mods::auth::AuthProviderRef;
use crate::mods::config::ConfigProviderRef;
use crate::mods::data::application::services::{
    HostService, ProfileService, StreamingQueryService, XcloudService,
};
use crate::mods::data::domain::DataSessionContext;
use crate::mods::data::infrastructure::XboxWebApiProvider;
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostSummary, DataSendTextResult, DataStreamingTitleInputConfig,
    DataUserProfile, DataXcloudTitleSummary,
};
use crate::mods::data::DataProvider;
use async_trait::async_trait;
use tokio::sync::Mutex;

pub struct DataService {
    auth_provider: AuthProviderRef,
    // 仅保护 WebApi client 复用状态，避免跨 data 域请求被串行阻塞。
    web_api_provider: Mutex<XboxWebApiProvider>,
    host_service: HostService,
    profile_service: ProfileService,
    streaming_query_service: StreamingQueryService,
    // xcloud 服务内部有内存缓存，需要可变访问；单独加锁，避免影响 profile/hosts。
    xcloud_service: Mutex<XcloudService>,
}

impl DataService {
    pub fn new(
        app_handle: tauri::AppHandle,
        auth_provider: AuthProviderRef,
        config_provider: ConfigProviderRef,
    ) -> Self {
        Self {
            auth_provider,
            web_api_provider: Mutex::new(XboxWebApiProvider::new()),
            host_service: HostService::new(),
            profile_service: ProfileService::new(app_handle.clone()),
            streaming_query_service: StreamingQueryService::new(config_provider),
            xcloud_service: Mutex::new(XcloudService::new(app_handle)),
        }
    }
}

#[async_trait]
impl DataProvider for DataService {
    async fn get_user_profile(&self) -> Result<DataUserProfile, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            self.profile_service.clear_cached_profile()?;
            return self.profile_service.get_cached_profile(0);
        };

        if let Some(web_api) = self.resolve_web_api_client(&session).await {
            let _ = self
                .profile_service
                .refresh_profile(&session, &web_api)
                .await;
        }

        self.profile_service.get_cached_profile(session.app_level)
    }

    async fn get_hosts(&self) -> Result<Vec<DataHostSummary>, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            eprintln!("[data][hosts] skip: no authenticated session");
            return Ok(Vec::new());
        };

        let Some(web_api) = self.resolve_web_api_client(&session).await else {
            eprintln!("[data][hosts] skip: web api client unavailable");
            return Ok(Vec::new());
        };

        let hosts = self.host_service.get_hosts(&session, &web_api).await?;
        eprintln!("[data][hosts] service result count={}", hosts.len());
        Ok(hosts)
    }

    async fn get_remote_consoles(&self) -> Result<Vec<DataHostSummary>, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(Vec::new());
        };

        self.streaming_query_service
            .get_remote_consoles(&session)
            .await
    }

    async fn get_streaming_title_input_config(
        &self,
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

    async fn power_on_console(&self, console_id: &str) -> Result<DataConsolePowerResult, String> {
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

    async fn power_off_console(&self, console_id: &str) -> Result<DataConsolePowerResult, String> {
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

    async fn send_text_to_console(
        &self,
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

    async fn get_xcloud_titles(&self) -> Result<Vec<DataXcloudTitleSummary>, String> {
        let session = self.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(Vec::new());
        };

        let mut xcloud_service = self.xcloud_service.lock().await;
        xcloud_service.get_titles(&session).await
    }
}

impl DataService {
    async fn ensure_authenticated_session(&self) -> Result<Option<DataSessionContext>, String> {
        if let Some(event) = self
            .auth_provider
            .get_active_session()
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(DataSessionContext {
                provider: event.provider,
                app_level: event.app_level,
                streaming_tokens: event.streaming_tokens,
                web_token: event.web_token,
            }));
        }

        let _ = self
            .auth_provider
            .check_authentication()
            .await
            .map_err(|error| error.to_string())?;
        if let Some(event) = self
            .auth_provider
            .get_active_session()
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(DataSessionContext {
                provider: event.provider,
                app_level: event.app_level,
                streaming_tokens: event.streaming_tokens,
                web_token: event.web_token,
            }));
        }

        Ok(None)
    }

    async fn resolve_web_api_client(
        &self,
        session: &DataSessionContext,
    ) -> Option<std::sync::Arc<crate::mods::data::client::XboxWebApiClient>> {
        let mut web_api_provider = self.web_api_provider.lock().await;
        web_api_provider.get_or_create(session)
    }
}
