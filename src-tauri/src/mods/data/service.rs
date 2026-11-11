#[path = "cloud_queries.rs"]
mod cloud_queries;
use crate::mods::auth::{AuthProviderRef, AuthTokenScope};
use crate::mods::config::ConfigProviderRef;
use crate::mods::data::cache_repository::DataCacheRepository;
use crate::mods::data::events;
use crate::mods::data::runtime_state::{DataRuntimeState, XcloudCatalogRefreshJoin};
use crate::mods::data::services::personal_service::{PersonalContext, PersonalService};
use crate::mods::data::services::{
    HostService, ProfileService, StreamingQueryService, XcloudService,
};
use crate::mods::data::session_resolver::DataSessionResolver;
use crate::mods::data::types::{
    DataConsoleLibrary, DataMediaSaveResult, DataPersonalAccount, DataPlaytime, DataQuery,
    LibraryAchievement, LibraryGame, LibraryMedia, LibraryPage, MediaKind,
};
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostSummary, DataSendTextResult, DataStreamingTitleInputConfig,
    DataUserProfile, DataXcloudCatalogCacheState, DataXcloudCatalogPayload,
    DataXcloudCatalogUpdatedEvent, XcloudCatalogCacheScope,
};
use crate::mods::data::DataError;
use crate::mods::data::DataProvider;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use async_trait::async_trait;
use std::sync::Arc;

pub struct DataService {
    app_handle: tauri::AppHandle,
    config_provider: ConfigProviderRef,
    session_resolver: DataSessionResolver,
    cache_repository: Arc<DataCacheRepository>,
    runtime_state: Arc<DataRuntimeState>,
    cloud_queries: cloud_queries::CloudQueryState,
    host_service: HostService,
    profile_service: ProfileService,
    streaming_query_service: StreamingQueryService,
    xcloud_service: Arc<XcloudService>,
    runtime_trace: RuntimeTraceRecorderRef,
}

struct XcloudRefreshContext {
    app_handle: tauri::AppHandle,
    xcloud_service: Arc<XcloudService>,
    cache_repository: Arc<DataCacheRepository>,
    session_resolver: DataSessionResolver,
    config_provider: ConfigProviderRef,
    runtime_trace: RuntimeTraceRecorderRef,
}

impl DataService {
    async fn personal_query<T, F, Fut>(
        &self,
        operation: &'static str,
        target: Option<&str>,
        request: F,
    ) -> Result<DataQuery<T>, DataError>
    where
        T: Send,
        F: Fn(PersonalContext) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<T, xbox_webapi::WebApiError>> + Send,
    {
        let started = std::time::Instant::now();
        let request = &request;
        let result = self
            .session_resolver
            .run_query(|session| async move {
                let ctx =
                    PersonalService::context(&session, target, self.runtime_trace.clone()).await?;
                let data = request(ctx.clone()).await?;
                Ok(ctx.wrap(data))
            })
            .await;
        self.record_http_result(operation, started, result.as_ref().err());
        result.map_err(DataError::from)
    }

    fn record_http_result(
        &self,
        operation: &'static str,
        started: std::time::Instant,
        error: Option<&xbox_webapi::WebApiError>,
    ) {
        record_data_http_result(&self.runtime_trace, operation, started, error);
    }

    pub fn new(
        app_handle: tauri::AppHandle,
        auth_provider: AuthProviderRef,
        config_provider: ConfigProviderRef,
        runtime_trace: RuntimeTraceRecorderRef,
    ) -> Self {
        let cache_repository = Arc::new(DataCacheRepository::new(app_handle.clone()));
        Self {
            app_handle,
            config_provider: config_provider.clone(),
            session_resolver: DataSessionResolver::new(auth_provider),
            cache_repository,
            runtime_state: Arc::new(DataRuntimeState::new()),
            cloud_queries: cloud_queries::CloudQueryState::new(),
            host_service: HostService::new(),
            profile_service: ProfileService::new(),
            streaming_query_service: StreamingQueryService::new(
                config_provider.clone(),
                runtime_trace.clone(),
            ),
            xcloud_service: Arc::new(XcloudService::with_config(config_provider.clone())),
            runtime_trace,
        }
    }

    fn catalog_scope(&self) -> Result<xbox_cloud_catalog_flow::CloudCatalogScope, DataError> {
        Self::catalog_scope_for(&self.config_provider)
    }

    fn catalog_scope_for(
        config_provider: &ConfigProviderRef,
    ) -> Result<xbox_cloud_catalog_flow::CloudCatalogScope, DataError> {
        let config = config_provider.get_by_keys(&["locale".to_owned()])?;
        let locale = config
            .get("locale")
            .and_then(|value| value.as_str())
            .unwrap_or("en");
        Ok(XcloudService::resolve_catalog_scope(locale))
    }

    async fn resolve_xcloud_catalog_context(
        &self,
    ) -> Result<
        Option<(
            crate::mods::data::types::DataSessionContext,
            XcloudCatalogCacheScope,
        )>,
        DataError,
    > {
        let started = std::time::Instant::now();
        let result = self.resolve_xcloud_catalog_context_inner().await;
        self.record_http_result("xcloudAccess", started, result.as_ref().err());
        result.map_err(catalog_data_error)
    }

    async fn resolve_xcloud_catalog_context_inner(
        &self,
    ) -> Result<
        Option<(
            crate::mods::data::types::DataSessionContext,
            XcloudCatalogCacheScope,
        )>,
        xbox_webapi::WebApiError,
    > {
        let session = self.session_resolver.current_session()?.unwrap_or(
            crate::mods::data::types::DataSessionContext {
                provider: "xbox".into(),
                app_level: 0,
                web_token: serde_json::json!({}),
                streaming_tokens: serde_json::json!({}),
            },
        );
        let catalog_scope = self
            .catalog_scope()
            .map_err(|_| xbox_webapi::WebApiError::parse("invalid catalog configuration"))?;
        let scope = self
            .xcloud_service
            .resolve_cache_scope(&session, &catalog_scope)
            .unwrap_or(XcloudCatalogCacheScope {
                account_id: "anonymous".into(),
                region_host: "public".into(),
                market: catalog_scope.market,
                language: catalog_scope.language,
            });

        Ok(Some((session, scope)))
    }

    async fn get_xcloud_catalog_impl(
        &self,
        reason: &str,
    ) -> Result<DataXcloudCatalogPayload, DataError> {
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
                self.spawn_xcloud_refresh(session, scope.clone(), reason.into(), false);
                Ok(DataXcloudCatalogPayload {
                    sources: Default::default(),
                    access: if scope.region_host == "public" {
                        "browse"
                    } else {
                        "account"
                    }
                    .into(),
                    error: None,
                    scope_key: Some(cache_key),
                    titles: Vec::new(),
                    cache_state: DataXcloudCatalogCacheState::Miss,
                    updated_at: None,
                    refreshing: true,
                })
            }
            DataXcloudCatalogCacheState::Fresh | DataXcloudCatalogCacheState::Stale => {
                let payload = DataXcloudCatalogPayload {
                    sources: self
                        .cache_repository
                        .read_cloud_value(&format!("{}.status", cache_key))?
                        .unwrap_or_default(),
                    access: if scope.region_host == "public" {
                        "browse"
                    } else {
                        "account"
                    }
                    .into(),
                    error: None,

                    scope_key: Some(cache_key.clone()),
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
    ) -> Result<DataXcloudCatalogPayload, DataError> {
        let Some((session, scope)) = self.resolve_xcloud_catalog_context().await? else {
            return Ok(Self::empty_catalog_payload(
                DataXcloudCatalogCacheState::Miss,
            ));
        };

        self.await_xcloud_refresh(session, scope, reason, true)
            .await
    }

    async fn prime_xcloud_catalog_impl(&self, reason: &str) -> Result<bool, DataError> {
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
    ) -> Result<DataXcloudCatalogPayload, DataError> {
        let cache_key = DataCacheRepository::scoped_cache_key(&scope);
        match self.runtime_state.begin_xcloud_refresh(&cache_key) {
            XcloudCatalogRefreshJoin::Leader(lease) => {
                let result = Self::perform_xcloud_refresh_task(
                    self.xcloud_refresh_context(),
                    session,
                    scope,
                    reason.to_string(),
                    force_refresh,
                )
                .await;
                lease.finish(result.clone());
                result
            }
            XcloudCatalogRefreshJoin::Follower(receiver) => receiver
                .await
                .map_err(|_| "目录刷新已中断，请重试".to_string())?,
        }
    }

    fn xcloud_refresh_context(&self) -> XcloudRefreshContext {
        XcloudRefreshContext {
            app_handle: self.app_handle.clone(),
            xcloud_service: self.xcloud_service.clone(),
            cache_repository: self.cache_repository.clone(),
            session_resolver: self.session_resolver.clone(),
            config_provider: self.config_provider.clone(),
            runtime_trace: self.runtime_trace.clone(),
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
        if let XcloudCatalogRefreshJoin::Leader(lease) =
            self.runtime_state.begin_xcloud_refresh(&cache_key)
        {
            let context = self.xcloud_refresh_context();
            tauri::async_runtime::spawn(async move {
                let result = Self::perform_xcloud_refresh_task(
                    context,
                    session,
                    scope,
                    reason,
                    force_refresh,
                )
                .await;
                lease.finish(result);
            });
        }
    }

    async fn perform_xcloud_refresh_task(
        context: XcloudRefreshContext,
        _session: crate::mods::data::types::DataSessionContext,
        scope: XcloudCatalogCacheScope,
        reason: String,
        force_refresh: bool,
    ) -> Result<DataXcloudCatalogPayload, DataError> {
        let started_at = std::time::Instant::now();
        let emit = |mut payload: DataXcloudCatalogPayload| {
            // 后台结果只发布到仍然有效的账户、语言和区域范围。
            let current = context.session_resolver.current_session().ok().flatten();
            let current_catalog = Self::catalog_scope_for(&context.config_provider).ok();
            let same = current_catalog
                .as_ref()
                .is_some_and(|c| c.market == scope.market && c.language == scope.language)
                && if scope.account_id == "anonymous" {
                    current.is_none()
                } else {
                    current
                        .as_ref()
                        .and_then(|session| {
                            context
                                .xcloud_service
                                .resolve_cache_scope(session, current_catalog.as_ref()?)
                        })
                        .as_ref()
                        == Some(&scope)
                };
            if !same {
                return;
            }
            let event = DataXcloudCatalogUpdatedEvent {
                scope_key: DataCacheRepository::scoped_cache_key(&scope),
                titles: std::mem::take(&mut payload.titles),
                cache_state: payload.cache_state,
                updated_at: payload.updated_at,
                refreshing: payload.refreshing,
                sources: payload.sources,
                access: payload.access,
                error: payload.error,
                reason: reason.clone(),
            };
            let _ = events::emit_xcloud_catalog_updated(&context.app_handle, &event);
        };
        let result = if scope.region_host == "public" {
            context
                .xcloud_service
                .refresh_catalog(
                    &_session,
                    &scope,
                    &context.cache_repository,
                    force_refresh,
                    &emit,
                )
                .await
        } else {
            context
                .session_resolver
                .run_scoped_query(AuthTokenScope::Cloud, |session| {
                    let emit = &emit;
                    let context = &context;
                    let scope = &scope;
                    async move {
                        let catalog =
                            Self::catalog_scope_for(&context.config_provider).map_err(|_| {
                                xbox_webapi::WebApiError::parse("invalid catalog scope")
                            })?;
                        if context
                            .xcloud_service
                            .resolve_cache_scope(&session, &catalog)
                            .as_ref()
                            != Some(scope)
                        {
                            return Err(xbox_webapi::WebApiError::parse(
                                "登录或目录设置已更新，请刷新",
                            ));
                        }
                        context
                            .xcloud_service
                            .refresh_catalog(
                                &session,
                                scope,
                                &context.cache_repository,
                                force_refresh,
                                emit,
                            )
                            .await
                    }
                })
                .await
        };
        record_data_http_result(
            &context.runtime_trace,
            "cloudCatalog",
            started_at,
            result.as_ref().err(),
        );
        match result {
            Ok(outcome) => {
                log::debug!(
                    "[Data][xcloud] hydrated candidates={}",
                    outcome.missing_product_count
                );
                emit(outcome.payload.clone());
                Ok(outcome.payload)
            }
            Err(error) => {
                let loaded = context
                    .cache_repository
                    .load_xcloud_catalog(&scope)
                    .map_err(DataError::from)?;
                if loaded.titles.is_empty() && scope.region_host != "public" {
                    let public_scope = XcloudCatalogCacheScope {
                        region_host: "public".into(),
                        ..scope.clone()
                    };
                    let progress = |mut payload: DataXcloudCatalogPayload| {
                        payload.scope_key = Some(DataCacheRepository::scoped_cache_key(&scope));
                        emit(payload);
                    };
                    if let Ok(mut outcome) = context
                        .xcloud_service
                        .refresh_catalog(
                            &_session,
                            &public_scope,
                            &context.cache_repository,
                            force_refresh,
                            progress,
                        )
                        .await
                    {
                        outcome.payload.scope_key =
                            Some(DataCacheRepository::scoped_cache_key(&scope));
                        outcome.payload.error = Some(XcloudService::catalog_error(&error));
                        emit(outcome.payload.clone());
                        return Ok(outcome.payload);
                    }
                }
                let payload = DataXcloudCatalogPayload {
                    scope_key: Some(DataCacheRepository::scoped_cache_key(&scope)),
                    titles: loaded.titles,
                    cache_state: loaded.cache_state,
                    updated_at: loaded.updated_at,
                    refreshing: false,
                    access: if scope.region_host == "public" {
                        "browse"
                    } else {
                        "unavailable"
                    }
                    .into(),
                    sources: context
                        .cache_repository
                        .read_cloud_value(&format!(
                            "{}.status",
                            DataCacheRepository::scoped_cache_key(&scope)
                        ))?
                        .unwrap_or_default(),
                    error: Some(XcloudService::catalog_error(&error)),
                };
                emit(payload.clone());
                Ok(payload)
            }
        }
    }

    fn empty_catalog_payload(cache_state: DataXcloudCatalogCacheState) -> DataXcloudCatalogPayload {
        DataXcloudCatalogPayload {
            sources: Default::default(),
            access: String::new(),
            error: None,

            scope_key: None,
            titles: Vec::new(),
            cache_state,
            updated_at: None,
            refreshing: false,
        }
    }
}

#[async_trait]
impl DataProvider for DataService {
    async fn validate_cloud_stream(&self, stream: &str) -> Result<(), DataError> {
        let title = self.cloud_title(stream, "titleId").await?;
        if !title.entitlement_known {
            return Err("游玩资格待确认，请刷新后重试".into());
        }
        if !title.has_entitlement {
            return Err("当前账户缺少游玩资格".into());
        }
        Ok(())
    }

    async fn get_cloud_detail(
        &self,
        product: &str,
    ) -> Result<super::types::DataCloudDetail, DataError> {
        self.cloud_detail(product).await
    }
    async fn get_cloud_preferences(
        &self,
    ) -> Result<(String, super::types::DataCloudPreferences), DataError> {
        self.cloud_preferences()
    }
    async fn set_cloud_preferences(
        &self,
        scope: &str,
        preferences: super::types::DataCloudPreferences,
    ) -> Result<super::types::DataCloudPreferences, DataError> {
        self.save_cloud_preferences(scope, preferences)
    }
    async fn resolve_cloud_title(
        &self,
        product: &str,
    ) -> Result<super::types::DataXcloudTitleSummary, DataError> {
        self.cloud_title(product, "productId").await
    }
    async fn get_cloud_wait(
        &self,
        product: &str,
        request: &str,
    ) -> Result<super::types::DataCloudWait, DataError> {
        self.cloud_wait(product, request).await
    }
    async fn cancel_cloud_wait_query(&self, request: &str) -> Result<bool, DataError> {
        self.cancel_cloud_wait(request).await;
        Ok(true)
    }
    async fn get_cloud_input_config(
        &self,
        stream: &str,
    ) -> Result<DataStreamingTitleInputConfig, DataError> {
        self.cloud_input(stream).await
    }

    async fn get_library_game(
        &self,
        target: Option<String>,
        title: String,
    ) -> Result<DataQuery<Option<LibraryGame>>, DataError> {
        self.personal_query("titleDetail", target.as_deref(), |ctx| {
            let title = title.clone();
            async move { PersonalService::game(&ctx, &title).await }
        })
        .await
    }

    async fn get_personal_account(
        &self,
        target: Option<String>,
    ) -> Result<DataQuery<DataPersonalAccount>, DataError> {
        self.personal_query("personalAccount", target.as_deref(), |ctx| async move {
            PersonalService::account(&ctx).await
        })
        .await
    }
    async fn get_game_history(
        &self,
        target: Option<String>,
        continuation: Option<String>,
    ) -> Result<DataQuery<LibraryPage<LibraryGame>>, DataError> {
        self.personal_query("titleHistory", target.as_deref(), |ctx| {
            let continuation = continuation.clone();
            async move { PersonalService::history(&ctx, continuation.as_deref()).await }
        })
        .await
    }
    async fn get_achievements(
        &self,
        target: Option<String>,
        title: String,
        continuation: Option<String>,
        language: String,
    ) -> Result<DataQuery<LibraryPage<LibraryAchievement>>, DataError> {
        self.personal_query("achievements", target.as_deref(), |ctx| {
            let (title, continuation, language) =
                (title.clone(), continuation.clone(), language.clone());
            async move {
                PersonalService::achievements(&ctx, &title, continuation.as_deref(), &language)
                    .await
            }
        })
        .await
    }
    async fn get_playtimes(
        &self,
        target: Option<String>,
        ids: Vec<String>,
    ) -> Result<DataQuery<Vec<DataPlaytime>>, DataError> {
        self.personal_query("playtimes", target.as_deref(), |ctx| {
            let ids = ids.clone();
            async move { PersonalService::playtimes(&ctx, ids).await }
        })
        .await
    }
    async fn get_console_library(
        &self,
        id: String,
    ) -> Result<DataQuery<DataConsoleLibrary>, DataError> {
        let scope = self.catalog_scope()?;
        self.personal_query("consoleLibrary", None, |ctx| {
            let (id, scope) = (id.clone(), scope.clone());
            async move { PersonalService::console(&ctx, &id, &scope).await }
        })
        .await
    }
    async fn get_media(
        &self,
        kind: MediaKind,
        continuation: Option<String>,
    ) -> Result<DataQuery<LibraryPage<LibraryMedia>>, DataError> {
        self.personal_query("mediaSearch", None, |ctx| {
            let continuation = continuation.clone();
            async move { PersonalService::media(&ctx, kind, continuation.as_deref()).await }
        })
        .await
    }
    async fn resolve_media(
        &self,
        kind: MediaKind,
        id: String,
        continuation: Option<String>,
    ) -> Result<DataQuery<LibraryMedia>, DataError> {
        self.personal_query("mediaResolve", None, |ctx| {
            let (id, continuation) = (id.clone(), continuation.clone());
            async move {
                crate::mods::data::services::media_save::resolve(
                    &ctx,
                    kind,
                    &id,
                    continuation.as_deref(),
                )
                .await
            }
        })
        .await
    }
    async fn save_media(
        &self,
        kind: MediaKind,
        id: String,
        continuation: Option<String>,
    ) -> Result<DataMediaSaveResult, DataError> {
        crate::mods::data::services::media_save::save(
            &self.session_resolver,
            &self.runtime_trace,
            kind,
            &id,
            continuation.as_deref(),
        )
        .await
    }

    async fn get_user_profile(&self) -> Result<DataUserProfile, DataError> {
        let started = std::time::Instant::now();
        let result = self
            .session_resolver
            .run_query(|session| async move {
                let profile = self.profile_service.fetch_profile(&session).await?;
                Ok((session, profile))
            })
            .await;
        self.record_http_result("profile", started, result.as_ref().err());
        match result {
            Ok((session, profile)) => {
                if let Some(claims) = crate::mods::data::session_resolver::resolve_web_token_claims(
                    &session.web_token,
                ) {
                    if let Err(error) = self
                        .cache_repository
                        .save_cached_profile(claims.account_scope(), &profile)
                    {
                        log::warn!("[Data] save profile cache failed: {}", error);
                    }
                }
            }
            Err(error)
                if matches!(error, xbox_webapi::WebApiError::Auth { .. })
                    || error.to_status_code() == Some(401) =>
            {
                return Err(error.into());
            }
            Err(error) => log::warn!("[Data] refresh profile failed: {}", error.safe_summary()),
        }
        let current = self
            .session_resolver
            .current_session()
            .map_err(DataError::from)?;
        let Some(current) = current else {
            self.cache_repository.clear_cached_profile()?;
            return self
                .cache_repository
                .get_cached_profile(None, 0)
                .map_err(Into::into);
        };
        let claims =
            crate::mods::data::session_resolver::resolve_web_token_claims(&current.web_token);
        self.cache_repository
            .get_cached_profile(
                claims.as_ref().map(|claims| claims.account_scope()),
                current.app_level,
            )
            .map_err(Into::into)
    }

    async fn get_hosts(&self) -> Result<Vec<DataHostSummary>, DataError> {
        let started = std::time::Instant::now();
        let result = self
            .session_resolver
            .run_query(|session| async move { self.host_service.get_hosts(&session).await })
            .await;
        self.record_http_result("hosts", started, result.as_ref().err());
        result.map_err(DataError::from)
    }

    async fn get_remote_consoles(&self) -> Result<Vec<DataHostSummary>, DataError> {
        self.session_resolver
            .run_scoped_query(
                crate::mods::auth::AuthTokenScope::Home,
                |session| async move {
                    self.streaming_query_service
                        .discover_remote_consoles(&session, None, || {
                            self.session_resolver
                                .is_current_streaming_session(&session, AuthTokenScope::Home)
                        })
                        .await
                },
            )
            .await
            .map_err(DataError::from)
    }

    async fn find_remote_console(
        &self,
        target: &str,
    ) -> Result<Option<DataHostSummary>, DataError> {
        self.session_resolver
            .run_scoped_query(
                crate::mods::auth::AuthTokenScope::Home,
                |session| async move {
                    let items = self
                        .streaming_query_service
                        .discover_remote_consoles(&session, Some(target), || {
                            self.session_resolver
                                .is_current_streaming_session(&session, AuthTokenScope::Home)
                        })
                        .await?;
                    Ok(items.into_iter().find(|item| {
                        [&item.server_id, &item.id, &item.device_id]
                            .into_iter()
                            .flatten()
                            .any(|identity| identity == target)
                    }))
                },
            )
            .await
            .map_err(DataError::from)
    }
    async fn get_streaming_regions(
        &self,
        target: xbox_streaming::Target,
    ) -> Result<super::types::DataStreamingRegions, DataError> {
        let scope = if target.is_home() {
            crate::mods::auth::AuthTokenScope::Home
        } else {
            crate::mods::auth::AuthTokenScope::Cloud
        };
        let started = std::time::Instant::now();
        let result = self
            .session_resolver
            .run_scoped_query(scope, |session| async move {
                self.streaming_query_service
                    .get_streaming_regions(&session, target)
            })
            .await;
        self.record_http_result(
            if target.is_home() {
                "homeRegions"
            } else {
                "cloudRegions"
            },
            started,
            result.as_ref().err(),
        );
        result.map_err(catalog_data_error)
    }

    async fn get_streaming_title_input_config(
        &self,
        xbox_title_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, DataError> {
        self.session_resolver
            .run_scoped_query(
                crate::mods::auth::AuthTokenScope::Home,
                |session| async move {
                    self.streaming_query_service
                        .get_streaming_title_input_config(&session, xbox_title_id)
                        .await
                },
            )
            .await
            .map_err(DataError::from)
    }

    async fn power_on_console(
        &self,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, DataError> {
        self.session_resolver
            .run_query(|session| async move {
                self.streaming_query_service
                    .power_on_console(&session, console_id)
                    .await
            })
            .await
            .map_err(DataError::from)
    }

    async fn power_off_console(
        &self,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, DataError> {
        self.session_resolver
            .run_query(|session| async move {
                self.streaming_query_service
                    .power_off_console(&session, console_id)
                    .await
            })
            .await
            .map_err(DataError::from)
    }

    async fn send_text_to_console(
        &self,
        console_id: &str,
        text: &str,
    ) -> Result<DataSendTextResult, DataError> {
        self.session_resolver
            .run_query(|session| async move {
                self.streaming_query_service
                    .send_text_to_console(&session, console_id, text)
                    .await
            })
            .await
            .map_err(DataError::from)
    }

    async fn get_xcloud_titles(&self) -> Result<DataXcloudCatalogPayload, DataError> {
        self.get_xcloud_catalog_impl("pageEnter").await
    }

    async fn refresh_xcloud_titles(&self) -> Result<DataXcloudCatalogPayload, DataError> {
        self.refresh_xcloud_catalog_impl("manualRefresh").await
    }

    async fn prime_xcloud_titles(&self) -> Result<bool, DataError> {
        self.prime_xcloud_catalog_impl("startupWarmup").await
    }
}

fn catalog_data_error(error: xbox_webapi::WebApiError) -> DataError {
    let message = XcloudService::catalog_error(&error);
    DataError::request(error, message)
}

fn record_data_http_result(
    recorder: &RuntimeTraceRecorderRef,
    operation: &'static str,
    started: std::time::Instant,
    error: Option<&xbox_webapi::WebApiError>,
) {
    recorder.record_event(
        "data",
        "httpQueryFinished",
        None,
        serde_json::json!({
            "operation": operation,
            "elapsedMs": started.elapsed().as_millis(),
            "outcome": if error.is_some() { "failed" } else { "success" },
            "error": error.map(xbox_webapi::WebApiError::safe_summary),
        }),
    );
}

#[cfg(test)]
#[path = "service.test.rs"]
mod tests;
