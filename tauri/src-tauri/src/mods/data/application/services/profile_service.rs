use crate::mods::data::client::XboxWebApiClient;
use crate::mods::data::domain::DataSessionContext;
use crate::mods::data::types::DataUserProfile;
use serde_json::{Map, Value};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

const PROFILE_CACHE_KEY: &str = "data.profileCache";

pub struct ProfileService {
    app_handle: AppHandle,
}

impl ProfileService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    pub async fn refresh_profile(
        &self,
        _session: &DataSessionContext,
        web_api: &XboxWebApiClient,
    ) -> Result<(), String> {
        println!("[DEBUG] refresh_profile: Calling web_api.get_current_user_profile()");
        let response = match web_api.get_current_user_profile().await {
            Ok(res) => res,
            Err(e) => {
                println!("[DEBUG] get_current_user_profile failed: {:?}", e);
                return Err(e);
            }
        };

        println!("[DEBUG] refresh_profile: parsing response");
        let root = response.get("data").unwrap_or(&response);
        let settings = root
            .get("profileUsers")
            .and_then(|value| value.as_array())
            .and_then(|users| users.first())
            .and_then(|first| first.get("settings"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        let mut profile_patch = Map::new();
        for entry in settings {
            let Some(id) = entry.get("id").and_then(|value| value.as_str()) else {
                continue;
            };
            let Some(value) = entry.get("value").and_then(|value| value.as_str()) else {
                continue;
            };
            profile_patch.insert(id.to_string(), Value::String(value.to_string()));
        }

        println!("[DEBUG] refresh_profile: writing to store");
        let store = self.app_handle.store("settings.json").map_err(|error| {
            println!("[DEBUG] get store err: {:?}", error);
            error.to_string()
        })?;
        store.set(PROFILE_CACHE_KEY, Value::Object(profile_patch));
        if let Err(e) = store.save() {
            println!("[DEBUG] store save err: {:?}", e);
            return Err(e.to_string());
        }
        println!("[DEBUG] refresh_profile: success");
        Ok(())
    }

    pub fn clear_cached_profile(&self) -> Result<(), String> {
        let store = self
            .app_handle
            .store("settings.json")
            .map_err(|error| error.to_string())?;
        store.delete(PROFILE_CACHE_KEY);
        store.save().map_err(|error| error.to_string())
    }

    pub fn get_cached_profile(&self, app_level: u32) -> Result<DataUserProfile, String> {
        let store = self
            .app_handle
            .store("settings.json")
            .map_err(|error| error.to_string())?;

        let settings_value = store.get(PROFILE_CACHE_KEY).unwrap_or(Value::Null);
        let mut settings: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        if let Some(object) = settings_value.as_object() {
            for (key, value) in object {
                if let Some(value_str) = value.as_str() {
                    settings.insert(key.to_string(), value_str.to_string());
                }
            }
        }

        let gamertag = settings.get("Gamertag").cloned().unwrap_or_default();

        Ok(DataUserProfile {
            signed_in: !gamertag.is_empty(),
            game_display_name: settings.get("GameDisplayName").cloned().unwrap_or_default(),
            game_display_pic_raw: settings
                .get("GameDisplayPicRaw")
                .cloned()
                .unwrap_or_default(),
            gamertag,
            gamerscore: settings.get("Gamerscore").cloned().unwrap_or_default(),
            settings,
            app_level,
        })
    }
}
