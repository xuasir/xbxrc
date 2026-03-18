use crate::mods::auth::AuthProviderRef;
use crate::mods::config::ConfigProviderRef;
use crate::mods::data::cache_repository::DataCacheRepository;
use crate::mods::data::events;
use crate::mods::data::runtime_state::{DataRuntimeState, XcloudCatalogRefreshJoin};
use crate::mods::data::services::{
    HostService, ProfileService, StreamingQueryService, XcloudService,
};
use crate::mods::data::session_resolver::DataSessionResolver;
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostSummary, DataSendTextResult, DataStreamingTitleInputConfig,
    DataUserProfile, DataXcloudCatalogCacheState, DataXcloudCatalogPayload,
    DataXcloudCatalogUpdatedEvent, XcloudCatalogCacheScope,
};
use crate::mods::data::DataProvider;
use async_trait::async_trait;
use std::sync::Arc;

pub struct DataService {
    app_handle: tauri::AppHandle,
    session_resolver: DataSessionResolver,
    cache_repository: Arc<DataCacheRepository>,
    runtime_state: Arc<DataRuntimeState>,
    host_service: HostService,
    profile_service: ProfileService,
    streaming_query_service: StreamingQueryService,
    xcloud_service: Arc<XcloudService>,
}

impl DataService {
    pub fn new(
        app_handle: tauri::AppHandle,
        auth_provider: AuthProviderRef,
        config_provider: ConfigProviderRef,
    ) -> Self {
        let cache_repository = Arc::new(DataCacheRepository::new(app_handle.clone()));
        Self {
            app_handle,
            session_resolver: DataSessionResolver::new(auth_provider),
            cache_repository,
            runtime_state: Arc::new(DataRuntimeState::new()),
            host_service: HostService::new(),
            profile_service: ProfileService::new(),
            streaming_query_service: StreamingQueryService::new(config_provider),
            xcloud_service: Arc::new(XcloudService::new()),
        }
    }

    async fn resolve_xcloud_catalog_context(
        &self,
    ) -> Result<
        Option<(
            crate::mods::data::types::DataSessionContext,
            XcloudCatalogCacheScope,
        )>,
        String,
    > {
        let session = self.session_resolver.ensure_authenticated_session().await?;
        let Some(session) = session else {
            return Ok(None);
        };

        let Some(scope) = self.xcloud_service.resolve_cache_scope(&session) else {
            return Ok(None);
        };

        Ok(Some((session, scope)))
    }

    async fn get_xcloud_catalog_impl(
        &self,
        reason: &str,
    ) -> Result<DataXcloudCatalogPayload, String> {
        let Some((session, scope)) = self.resolve_xcloud_catalog_context().await? else {
            return Ok(Self::empty_catalog_payload(
                DataXcloudCatalogCacheState::Miss,
            ));
        };

        let cache_key = DataCacheRepository::scoped_cache_key(&scope);
        let loaded_snapshot = self.cache_repository.load_xcloud_catalog(&scope)?;
        let is_refreshing = self.runtime_state.is_xcloud_refreshing(&cache_key);

        log::info!(
            "[Data][xcloud] serve cache_state={:?} hit_level={} refreshing={} needs_refresh={} titles={}",
            loaded_snapshot.cache_state,
            loaded_snapshot.hit_level,
            is_refreshing,
            loaded_snapshot.needs_refresh,
            loaded_snapshot.titles.len()
        );

        match loaded_snapshot.cache_state {
            DataXcloudCatalogCacheState::Miss => {
                self.await_xcloud_refresh(session, scope, reason, false)
                    .await
            }
            DataXcloudCatalogCacheState::Fresh | DataXcloudCatalogCacheState::Stale => {
                let payload = DataXcloudCatalogPayload {
                    titles: loaded_snapshot.titles.clone(),
                    cache_state: loaded_snapshot.cache_state.clone(),
                    updated_at: loaded_snapshot.updated_at,
                    refreshing: is_refreshing || loaded_snapshot.needs_refresh,
                };

                if loaded_snapshot.needs_refresh && !is_refreshing {
                    self.spawn_xcloud_refresh(session, scope, "staleRevalidate".to_string(), false);
                }

                Ok(payload)
            }
        }
    }

    async fn refresh_xcloud_catalog_impl(
        &self,
        reason: &str,
    ) -> Result<DataXcloudCatalogPayload, String> {
        let Some((session, scope)) = self.resolve_xcloud_catalog_context().await? else {
            return Ok(Self::empty_catalog_payload(
                DataXcloudCatalogCacheState::Miss,
            ));
        };

        self.await_xcloud_refresh(session, scope, reason, true)
            .await
    }

    async fn prime_xcloud_catalog_impl(&self, reason: &str) -> Result<bool, String> {
        let Some((session, scope)) = self.resolve_xcloud_catalog_context().await? else {
            return Ok(false);
        };

        let cache_key = DataCacheRepository::scoped_cache_key(&scope);
        let loaded_snapshot = self.cache_repository.load_xcloud_catalog(&scope)?;
        if matches!(
            loaded_snapshot.cache_state,
            DataXcloudCatalogCacheState::Fresh
        ) && !loaded_snapshot.needs_refresh
        {
            return Ok(false);
        }

        if self.runtime_state.is_xcloud_refreshing(&cache_key) {
            return Ok(false);
        }

        self.spawn_xcloud_refresh(session, scope, reason.to_string(), false);
        Ok(true)
    }

    async fn await_xcloud_refresh(
        &self,
        session: crate::mods::data::types::DataSessionContext,
        scope: XcloudCatalogCacheScope,
        reason: &str,
        force_refresh: bool,
    ) -> Result<DataXcloudCatalogPayload, String> {
        let cache_key = DataCacheRepository::scoped_cache_key(&scope);
        match self.runtime_state.begin_xcloud_refresh(&cache_key) {
            XcloudCatalogRefreshJoin::Leader(_) => {
                let result = self
                    .perform_xcloud_refresh(session, scope, reason.to_string(), force_refresh)
                    .await;
                self.runtime_state
                    .finish_xcloud_refresh(&cache_key, result.clone());
                result
            }
            XcloudCatalogRefreshJoin::Follower(receiver) => receiver
                .await
                .map_err(|_| "xcloud catalog refresh task canceled".to_string())?,
        }
    }

    fn spawn_xcloud_refresh(
        &self,
        session: crate::mods::data::types::DataSessionContext,
        scope: XcloudCatalogCacheScope,
        reason: String,
        force_refresh: bool,
    ) {
        let cache_key = DataCacheRepository::scoped_cache_key(&scope);
        match self.runtime_state.begin_xcloud_refresh(&cache_key) {
            XcloudCatalogRefreshJoin::Leader(_) => {
                let runtime_state = self.runtime_state.clone();
                let cache_repository = self.cache_repository.clone();
                let xcloud_service = self.xcloud_service.clone();
                let app_handle = self.app_handle.clone();

                tauri::async_runtime::spawn(async move {
                    let result = DataService::perform_xcloud_refresh_task(
                        app_handle,
                        xcloud_service,
                        cache_repository,
                        session,
                        scope,
                        reason,
                        force_refresh,
                    )
                    .await;
                    runtime_state.finish_xcloud_refresh(&cache_key, result);
                });
            }
            XcloudCatalogRefreshJoin::Follower(_) => {}
        }
    }

    async fn perform_xcloud_refresh(
        &self,
        session: crate::mods::data::types::DataSessionContext,
        scope: XcloudCatalogCacheScope,
        reason: String,
        force_refresh: bool,
    ) -> Result<DataXcloudCatalogPayload, String> {
        Self::perform_xcloud_refresh_task(
            self.app_handle.clone(),
            self.xcloud_service.clone(),
            self.cache_repository.clone(),
            session,
            scope,
            reason,
            force_refresh,
        )
        .await
    }

    async fn perform_xcloud_refresh_task(
        app_handle: tauri::AppHandle,
        xcloud_service: Arc<XcloudService>,
        cache_repository: Arc<DataCacheRepository>,
        session: crate::mods::data::types::DataSessionContext,
        scope: XcloudCatalogCacheScope,
        reason: String,
        force_refresh: bool,
    ) -> Result<DataXcloudCatalogPayload, String> {
        let started_at = std::time::Instant::now();
        let outcome = xcloud_service
            .refresh_catalog(&session, &scope, &cache_repository, force_refresh)
            .await?;
        let elapsed_ms = started_at.elapsed().as_millis();

        log::info!(
            "[Data][xcloud] refresh reason={} duration={}ms titles={} missingProductCount={}",
            reason,
            elapsed_ms,
            outcome.payload.titles.len(),
            outcome.missing_product_count
        );

        let event_payload = DataXcloudCatalogUpdatedEvent {
            titles: outcome.payload.titles.clone(),
            cache_state: outcome.payload.cache_state.clone(),
            updated_at: outcome.payload.updated_at,
            refreshing: false,
            reason,
        };
        if let Err(error) = events::emit_xcloud_catalog_updated(&app_handle, &event_payload) {
            log::warn!("[Data][xcloud] emit catalog updated failed: {}", error);
        }

        Ok(outcome.payload)
    }

    fn empty_catalog_payload(cache_state: DataXcloudCatalogCacheState) -> DataXcloudCatalogPayload {
        DataXcloudCatalogPayload {
            titles: Vec::new(),
            cache_state,
            updated_at: None,
            refreshing: false,
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

    async fn get_xcloud_titles(&self) -> Result<DataXcloudCatalogPayload, String> {
        self.get_xcloud_catalog_impl("pageEnter").await
    }

    async fn refresh_xcloud_titles(&self) -> Result<DataXcloudCatalogPayload, String> {
        self.refresh_xcloud_catalog_impl("manualRefresh").await
    }

    async fn prime_xcloud_titles(&self) -> Result<bool, String> {
        self.prime_xcloud_catalog_impl("startupWarmup").await
    }
}
