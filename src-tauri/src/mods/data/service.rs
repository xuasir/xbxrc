use crate::mods::auth::AuthProviderRef;
use crate::mods::config::ConfigProviderRef;
use crate::mods::data::cache_repository::DataCacheRepository;
use crate::mods::data::services::{
    HostService, ProfileService, StreamingQueryService, XcloudService,
};
use crate::mods::data::session_resolver::DataSessionResolver;
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostSummary, DataSendTextResult, DataStreamingTitleInputConfig,
    DataUserProfile, DataXcloudTitleSummary,
};
use crate::mods::data::DataProvider;
use async_trait::async_trait;
use std::sync::Arc;

pub struct DataService {
    session_resolver: DataSessionResolver,
    cache_repository: Arc<DataCacheRepository>,
    host_service: HostService,
    profile_service: ProfileService,
    streaming_query_service: StreamingQueryService,
    xcloud_service: XcloudService,
}

impl DataService {
    pub fn new(
        app_handle: tauri::AppHandle,
        auth_provider: AuthProviderRef,
        config_provider: ConfigProviderRef,
    ) -> Self {
        let cache_repository = Arc::new(DataCacheRepository::new(app_handle.clone()));
        Self {
            session_resolver: DataSessionResolver::new(auth_provider),
            cache_repository,
            host_service: HostService::new(),
            profile_service: ProfileService::new(),
            streaming_query_service: StreamingQueryService::new(config_provider),
            xcloud_service: XcloudService::new(),
        }
    }
}

#[async_trait]
impl DataProvider for DataService {
    async fn get_user_profile(&self) -> Result<DataUserProfile, String> {
        let session = self.session_resolver.ensure_authenticated_session().await?;
        let Some(session) = session else {
            self.cache_repository.clear_cached_profile()?;
            return self.cache_repository.get_cached_profile(0);
        };

        match tokio::time::timeout(
            std::time::Duration::from_secs(6),
            self.profile_service.fetch_profile(&session),
        )
        .await
        {
            Ok(Ok(profile)) => {
                if let Err(error) = self.cache_repository.save_cached_profile(&profile) {
                    log::warn!("[Data] save profile cache failed: {}", error);
                }
            }
            Ok(Err(error)) => {
                log::warn!(
                    "[Data] refresh profile failed, fallback to cached profile: {}",
                    error
                );
            }
            Err(_) => {
                log::warn!("[Data] refresh profile timeout, fallback to cached profile");
            }
        }

        self.cache_repository.get_cached_profile(session.app_level)
    }

    async fn get_hosts(&self) -> Result<Vec<DataHostSummary>, String> {
        let session = self.session_resolver.ensure_authenticated_session().await?;
        let Some(session) = session else {
            eprintln!("[data][hosts] skip: no authenticated session");
            return Ok(Vec::new());
        };

        let hosts = self.host_service.get_hosts(&session).await?;
        eprintln!("[data][hosts] service result count={}", hosts.len());
        Ok(hosts)
    }

    async fn get_remote_consoles(&self) -> Result<Vec<DataHostSummary>, String> {
        let session = self.session_resolver.ensure_authenticated_session().await?;
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
        let session = self.session_resolver.ensure_authenticated_session().await?;
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
        let session = self.session_resolver.ensure_authenticated_session().await?;
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
        let session = self.session_resolver.ensure_authenticated_session().await?;
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
        let session = self.session_resolver.ensure_authenticated_session().await?;
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
        let session = self.session_resolver.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(Vec::new());
        };

        self.xcloud_service
            .get_titles(&session, &self.cache_repository)
            .await
    }
}
