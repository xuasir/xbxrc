use crate::mods::data::types::DataXcloudCatalogUpdatedEvent;
use tauri::AppHandle;

pub const DATA_XCLOUD_CATALOG_UPDATED_CHANNEL: &str = "xbxrc:data:xcloud-catalog-updated";

pub fn emit_xcloud_catalog_updated(
    app_handle: &AppHandle,
    payload: &DataXcloudCatalogUpdatedEvent,
) -> Result<(), String> {
    crate::event_bridge::emit(app_handle, DATA_XCLOUD_CATALOG_UPDATED_CHANNEL, payload)
}
