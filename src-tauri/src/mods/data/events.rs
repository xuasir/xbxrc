use crate::mods::data::types::{DataHostsChangedEvent, DataXcloudCatalogUpdatedEvent};
use tauri::AppHandle;

pub const DATA_XCLOUD_CATALOG_UPDATED_CHANNEL: &str = "xbxrc:data:xcloud-catalog-updated";
pub const DATA_HOSTS_CHANGED_CHANNEL: &str = "xbxrc:data:hosts-changed";

pub fn emit_hosts_changed(
    app_handle: &AppHandle,
    payload: &DataHostsChangedEvent,
) -> Result<(), String> {
    crate::event_bridge::emit(app_handle, DATA_HOSTS_CHANGED_CHANNEL, payload)
}

pub fn emit_xcloud_catalog_updated(
    app_handle: &AppHandle,
    payload: &DataXcloudCatalogUpdatedEvent,
) -> Result<(), String> {
    crate::event_bridge::emit(app_handle, DATA_XCLOUD_CATALOG_UPDATED_CHANNEL, payload)
}
