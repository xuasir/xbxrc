use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

pub struct AuthConfigBridge {
    app_handle: AppHandle,
}

impl AuthConfigBridge {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    pub fn get_force_region_ip(&self) -> String {
        if let Ok(store) = self.app_handle.store("settings.json") {
            if let Some(value) = store.get("force_region_ip") {
                if let Some(region_ip) = value.as_str() {
                    return region_ip.trim().to_string();
                }
            }
        }

        String::new()
    }
}
