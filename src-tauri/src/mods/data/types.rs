use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSessionContext {
    pub provider: String,
    pub app_level: u32,
    pub streaming_tokens: Value,
    pub web_token: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataAuthState {
    pub provider: String,
    pub is_authenticating: bool,
    pub is_authenticated: bool,
    pub app_level: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataHostStorageDeviceSummary {
    pub storage_device_id: Option<String>,
    pub storage_device_name: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub free_space_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub total_space_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataHostAddr {
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataHostSummary {
    pub id: Option<String>,
    pub device_id: Option<String>,
    pub server_id: Option<String>,
    pub name: Option<String>,
    pub device_name: Option<String>,
    pub locale: Option<String>,
    pub region: Option<String>,
    pub power_state: Option<String>,
    pub console_type: Option<String>,
    pub digital_assistant_remote_control_enabled: Option<bool>,
    pub remote_management_enabled: Option<bool>,
    pub console_streaming_enabled: Option<bool>,
    pub wireless_warning: Option<bool>,
    pub out_of_home_warning: Option<bool>,
    pub storage_devices: Option<Vec<DataHostStorageDeviceSummary>>,
    pub console_addrs: Option<Vec<DataHostAddr>>,
    #[serde(default)]
    pub discovery_region_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataUserProfile {
    pub signed_in: bool,
    pub game_display_name: String,
    pub game_display_pic_raw: String,
    pub gamertag: String,
    pub gamerscore: String,
    pub settings: HashMap<String, String>,
    pub app_level: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataStreamingTitleInputConfig {
    pub xbox_title_id: String,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataConsolePowerResult {
    pub console_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSendTextResult {
    pub console_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataXcloudTitleSummary {
    #[serde(default)]
    pub entitlement_known: bool,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub recent_rank: Option<usize>,
    #[serde(default)]
    pub new_rank: Option<usize>,
    #[serde(default)]
    pub popular_rank: Option<usize>,
    #[serde(default)]
    pub leaving_rank: Option<usize>,
    pub id: String,
    pub name: String,
    pub product_id: String,
    pub title_id: String,
    pub xbox_title_id: Option<u64>,
    pub publisher_name: String,
    pub description: String,
    pub tile_image_url: String,
    pub poster_image_url: String,
    pub categories: Vec<String>,
    pub supported_input_types: Vec<String>,
    pub has_entitlement: bool,
    pub is_recently_played: bool,
    pub is_new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DataXcloudCatalogCacheState {
    Miss,
    Fresh,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataXcloudCatalogPayload {
    #[serde(default)]
    pub sources: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_key: Option<String>,
    pub titles: Vec<DataXcloudTitleSummary>,
    pub cache_state: DataXcloudCatalogCacheState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    pub refreshing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataXcloudCatalogUpdatedEvent {
    #[serde(default)]
    pub sources: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub error: Option<String>,
    pub scope_key: String,
    pub titles: Vec<DataXcloudTitleSummary>,
    pub cache_state: DataXcloudCatalogCacheState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    pub refreshing: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct XcloudCatalogCacheScope {
    pub account_id: String,
    pub region_host: String,
    pub language: String,
    pub market: String,
}

pub use xbox_webapi::{
    ConsoleStorage, InstalledGame, LibraryAchievement, LibraryGame, LibraryMedia, LibraryPage,
    LibraryPerson, MediaKind,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataQuery<T> {
    pub account_id: String,
    pub target_id: String,
    pub updated_at: u64,
    pub data: T,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataPersonalAccount {
    pub profile: Option<LibraryPerson>,
    pub friends: Option<Vec<LibraryPerson>>,
    pub notices: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataPlaytime {
    pub xbox_title_id: String,
    pub minutes: Option<u64>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataConsoleLibrary {
    pub console_id: String,
    pub games: Option<Vec<InstalledGame>>,
    pub storage: Option<Vec<ConsoleStorage>>,
    pub power_state: Option<String>,
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMediaSaveResult {
    pub saved: bool,
    pub canceled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataStreamingRegion {
    pub name: String,
    pub display_name: Option<String>,
    pub short_name: Option<String>,
    pub continent: Option<String>,
    pub is_default: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataStreamingRegions {
    pub regions: Vec<DataStreamingRegion>,
    pub updated_at: u64,
    pub preferred_region_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataCloudPreferences {
    pub favorites: Vec<DataXcloudTitleSummary>,
    pub pinned: Vec<String>,
    pub section: String,
    pub category: String,
    pub sort: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataCloudDetail {
    pub detail: xbox_cloud_catalog_flow::CloudProductDetail,
    pub updated_at: u64,
    pub stale: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataCloudWait {
    pub seconds: Option<u64>,
    pub updated_at: u64,
}
