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
    pub titles: Vec<DataXcloudTitleSummary>,
    pub cache_state: DataXcloudCatalogCacheState,
    pub updated_at: Option<u64>,
    pub refreshing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataXcloudCatalogUpdatedEvent {
    pub titles: Vec<DataXcloudTitleSummary>,
    pub cache_state: DataXcloudCatalogCacheState,
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
