use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{DataSessionContext, DataUserProfile};
use serde_json::Value;
use std::collections::HashMap;
use xbox_webapi::{library_image_url, ProfileApi, WebApiError};

pub struct ProfileService;

impl ProfileService {
    pub fn new() -> Self {
        Self
    }

    pub async fn fetch_profile(
        &self,
        session: &DataSessionContext,
    ) -> Result<DataUserProfile, WebApiError> {
        let Some(claims) = resolve_web_token_claims(&session.web_token) else {
            return Err(WebApiError::auth("登录信息暂不可用，请重新登录"));
        };
        let profile_api = ProfileApi::new(claims.uhs, claims.user_token);

        let response = profile_api.get_current_user().await?;
        let settings = extract_profile_settings(&response).unwrap_or_default();

        let mut settings_map = HashMap::new();
        for entry in settings {
            let Some(id) = entry.get("id").and_then(|value| value.as_str()) else {
                continue;
            };
            let Some(value) = entry.get("value").and_then(|value| value.as_str()) else {
                continue;
            };
            settings_map.insert(id.to_string(), value.to_string());
        }

        // 防御：若响应结构异常，不覆盖已有缓存，避免把已登录态资料写空。
        if settings_map.is_empty() {
            return Err(WebApiError::parse(
                "invalid profile response: empty settings",
            ));
        }

        Ok(DataUserProfile {
            signed_in: settings_map.contains_key("Gamertag"),
            game_display_name: settings_map
                .get("GameDisplayName")
                .cloned()
                .unwrap_or_default(),
            game_display_pic_raw: settings_map
                .get("GameDisplayPicRaw")
                .and_then(|url| library_image_url(Some(&Value::String(url.clone()))))
                .unwrap_or_default(),
            gamertag: settings_map.get("Gamertag").cloned().unwrap_or_default(),
            gamerscore: settings_map.get("Gamerscore").cloned().unwrap_or_default(),
            settings: settings_map,
            app_level: session.app_level,
        })
    }
}

fn extract_profile_settings(response: &Value) -> Option<Vec<Value>> {
    // 兼容两种结构：
    // 1) { profileUsers: [{ settings: [...] }] }
    // 2) { settings: [...] }（调用方已下钻到 profileUsers[0]）
    if let Some(settings) = response.get("settings").and_then(|value| value.as_array()) {
        return Some(settings.clone());
    }

    let root = response.get("data").unwrap_or(response);
    root.get("profileUsers")
        .and_then(|value| value.as_array())
        .and_then(|users| users.first())
        .and_then(|first| first.get("settings"))
        .and_then(|value| value.as_array())
        .cloned()
}
