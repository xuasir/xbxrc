pub mod cache_repository;
mod catalog_projection;
pub mod error;
pub mod events;
mod host_cache;
pub mod rpc;
pub mod runtime_state;
pub mod service;
pub mod services;
pub mod session_resolver;
pub mod types;

pub use error::DataError;
pub use service::DataService;
pub use types::*;

use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait DataProvider: Send + Sync {
    async fn get_cloud_catalog_status(&self) -> Result<DataCloudCatalogStatus, DataError>;
    async fn get_cloud_title_summary(
        &self,
        product: &str,
    ) -> Result<DataCloudTitleSummary, DataError>;
    async fn prioritize_cloud_titles(&self, products: Vec<String>) -> Result<bool, DataError>;
    async fn validate_cloud_stream(&self, _stream: &str) -> Result<(), DataError> {
        Ok(())
    }
    async fn get_cloud_detail(&self, _product: &str) -> Result<DataCloudDetail, DataError> {
        Err("云详情暂不可用".into())
    }
    async fn get_cloud_preferences(&self) -> Result<(String, DataCloudPreferences), DataError> {
        Ok((String::new(), Default::default()))
    }
    async fn set_cloud_preferences(
        &self,
        _scope: &str,
        _preferences: DataCloudPreferences,
    ) -> Result<DataCloudPreferences, DataError> {
        Err("收藏暂不可用".into())
    }
    async fn resolve_cloud_title(
        &self,
        _product: &str,
    ) -> Result<DataXcloudTitleSummary, DataError> {
        Err("云游戏访问暂不可用".into())
    }
    async fn get_cloud_wait(
        &self,
        _product: &str,
        _request: &str,
    ) -> Result<DataCloudWait, DataError> {
        Err("等待时间暂不可用".into())
    }
    async fn cancel_cloud_wait_query(&self, _request: &str) -> Result<bool, DataError> {
        Ok(true)
    }
    async fn get_cloud_input_config(
        &self,
        _stream: &str,
    ) -> Result<DataStreamingTitleInputConfig, DataError> {
        Err("云输入配置暂不可用".into())
    }

    async fn get_library_game(
        &self,
        target: Option<String>,
        title: String,
    ) -> Result<DataQuery<Option<LibraryGame>>, DataError>;
    async fn get_personal_account(
        &self,
        target: Option<String>,
    ) -> Result<DataQuery<DataPersonalAccount>, DataError>;
    async fn get_game_history(
        &self,
        target: Option<String>,
        continuation: Option<String>,
    ) -> Result<DataQuery<LibraryPage<LibraryGame>>, DataError>;
    async fn get_achievements(
        &self,
        target: Option<String>,
        title: String,
        continuation: Option<String>,
        language: String,
    ) -> Result<DataQuery<LibraryPage<LibraryAchievement>>, DataError>;
    async fn get_playtimes(
        &self,
        target: Option<String>,
        ids: Vec<String>,
    ) -> Result<DataQuery<Vec<DataPlaytime>>, DataError>;
    async fn get_console_library(
        &self,
        id: String,
    ) -> Result<DataQuery<DataConsoleLibrary>, DataError>;
    async fn get_media(
        &self,
        kind: MediaKind,
        continuation: Option<String>,
    ) -> Result<DataQuery<LibraryPage<LibraryMedia>>, DataError>;
    async fn resolve_media(
        &self,
        kind: MediaKind,
        id: String,
        continuation: Option<String>,
    ) -> Result<DataQuery<LibraryMedia>, DataError>;
    async fn save_media(
        &self,
        kind: MediaKind,
        id: String,
        continuation: Option<String>,
    ) -> Result<DataMediaSaveResult, DataError>;
    async fn get_user_profile(&self) -> Result<DataUserProfile, DataError>;
    async fn get_hosts(&self) -> Result<Vec<DataHostSummary>, DataError>;
    async fn get_hosts_snapshot(&self) -> Result<DataHostsSnapshot, DataError>;
    async fn refresh_hosts(&self) -> Result<DataHostsSnapshot, DataError>;
    async fn get_remote_consoles(&self) -> Result<Vec<DataHostSummary>, DataError>;
    async fn find_remote_console(
        &self,
        target: &str,
    ) -> Result<Option<DataHostSummary>, DataError> {
        Ok(self.get_remote_consoles().await?.into_iter().find(|item| {
            [&item.server_id, &item.id, &item.device_id]
                .into_iter()
                .flatten()
                .any(|identity| identity == target)
        }))
    }
    async fn get_streaming_regions(
        &self,
        _target: xbox_streaming::Target,
    ) -> Result<DataStreamingRegions, DataError> {
        Ok(DataStreamingRegions::default())
    }
    async fn get_streaming_title_input_config(
        &self,
        xbox_title_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, DataError>;
    async fn power_on_console(&self, console_id: &str)
        -> Result<DataConsolePowerResult, DataError>;
    async fn power_off_console(
        &self,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, DataError>;
    async fn send_text_to_console(
        &self,
        console_id: &str,
        text: &str,
    ) -> Result<DataSendTextResult, DataError>;
    async fn get_xcloud_titles(&self) -> Result<DataXcloudCatalogPayload, DataError>;
    async fn refresh_xcloud_titles(&self) -> Result<DataXcloudCatalogPayload, DataError>;
    async fn prime_xcloud_titles(&self) -> Result<bool, DataError>;
}

pub type DataProviderRef = Arc<dyn DataProvider>;
