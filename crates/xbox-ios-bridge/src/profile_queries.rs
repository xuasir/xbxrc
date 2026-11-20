use crate::{
    deserialize, extract_profile_settings, query_failure::query_error, resolve_web_token_claims,
    XboxBridgeError, XboxProfile,
};
use serde_json::Value;
use xbox_webapi::{PeopleApi, ProfileApi, SocialApi, UserPresenceApi};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum XboxProfileSource {
    Identity,
    Social,
    Friends,
    Presence,
}

/// 每个来源复用公共API；Swift按来源提交并保留其他来源的成功事实。
#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_profile_section(
    web_token_json: String,
    source: XboxProfileSource,
) -> Result<XboxProfile, XboxBridgeError> {
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let mut profile = XboxProfile {
        xuid: claims.xuid,
        gamertag: String::new(),
        display_name: String::new(),
        gamer_score: String::new(),
        display_picture_url: String::new(),
        presence_state: None,
        presence_device: None,
        current_title_name: None,
        rich_presence: None,
        followers_count: None,
        following_count: None,
        friend_count: None,
    };
    match source {
        XboxProfileSource::Identity => {
            let response = ProfileApi::new(claims.uhs, claims.token)
                .get_current_user()
                .await
                .map_err(query_error)?;
            let settings = extract_profile_settings(&response);
            if settings.is_empty() {
                return Err(XboxBridgeError::InvalidData(
                    "missing profile settings".into(),
                ));
            }
            profile.gamertag = settings.get("Gamertag").cloned().unwrap_or_default();
            profile.display_name = settings.get("GameDisplayName").cloned().unwrap_or_default();
            profile.gamer_score = settings.get("Gamerscore").cloned().unwrap_or_default();
            profile.display_picture_url = settings
                .get("GameDisplayPicRaw")
                .cloned()
                .unwrap_or_default();
        }
        XboxProfileSource::Social => {
            let summary = SocialApi::new(claims.uhs, claims.token)
                .get_summary()
                .await
                .map_err(query_error)?;
            profile.followers_count = summary.followers_count;
            profile.following_count = summary.following_count;
        }
        XboxProfileSource::Friends => {
            profile.friend_count = PeopleApi::new(claims.uhs, claims.token)
                .get_friends_count()
                .await
                .map_err(query_error)?;
        }
        XboxProfileSource::Presence => {
            let presence = UserPresenceApi::new(claims.uhs, claims.token)
                .get_current_user()
                .await
                .map_err(query_error)?;
            profile.presence_state = presence.state;
            profile.presence_device = presence.device;
            profile.current_title_name = presence.current_title_name;
            profile.rich_presence = presence.rich_presence;
        }
    }
    Ok(profile)
}
