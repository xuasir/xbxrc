use serde_json::Value;
use std::collections::HashMap;
use xbox_auth_flow::{
    AuthFlow, AuthFlowError, AuthFlowSeed, CompleteOAuthLoginInput, PendingOAuthLogin,
    StartOAuthLoginInput,
};
use xbox_webapi::{PeopleApi, ProfileApi, SocialApi, UserPresenceApi};

mod auth_session;
mod catalog_sources;
pub use auth_session::{
    XboxAuthCheckpoint, XboxAuthObserver, XboxAuthScope, XboxAuthStage, XboxStreamingAuthProvider,
};
mod cloud_access;
mod cloud_catalog;
mod cloud_queries;
mod data;
mod installed_queries;
mod personal;
mod personal_queries;
mod profile_queries;
mod query_failure;
mod streaming;
mod touch_layouts;

pub use touch_layouts::{
    fetch_community_touch_bundle, fetch_community_touch_bundles, fetch_official_touch_bundle,
    fetch_official_touch_bundles, XboxOfficialTouchBundle, XboxOfficialTouchBundleChoice,
    XboxSavedTouchLayout, XboxTouchLayoutAsset, XboxTouchLayoutChoice, XboxTouchLayoutLibrary,
    XboxTouchLayoutSnapshot,
};

pub use catalog_sources::fetch_catalog_source;
pub use cloud_access::{
    prepare_cloud_access, prepare_home_access, release_stream_access, stream_cloud_regions,
    CloudAccessResult, HomeAccessResult,
};
pub use cloud_catalog::{
    fetch_cloud_catalog, hydrate_cloud_catalog_page, XboxCloudCatalogMetadata,
    XboxCloudCatalogSnapshot, XboxCloudGame,
};
pub use cloud_queries::{
    fetch_cloud_product_detail, fetch_cloud_wait_time, resolve_cloud_game,
    XboxCloudLanguageSupport, XboxCloudProductDetail,
};
pub use data::{
    fetch_achievements, fetch_game_library, fetch_hosts, fetch_playtimes, power_off_console,
    power_on_console, XboxAchievement, XboxAchievementProgress, XboxAchievementRequirement,
    XboxConsolePowerResult, XboxGame, XboxHost, XboxHostStorageDevice, XboxPlaytime,
};
pub use installed_queries::{fetch_installed_section, XboxInstalledSection, XboxInstalledSource};
pub use personal::{
    fetch_friends, fetch_installed_library, fetch_media_page, XboxConsoleStorage, XboxFriend,
    XboxInstalledGame, XboxInstalledLibrary, XboxMediaItem, XboxMediaPage,
};
pub use personal_queries::{
    fetch_achievement_page, fetch_game_history_page, fetch_library_game, fetch_player_playtimes,
    fetch_player_profile, XboxAchievementPage, XboxGameHistoryPage, XboxLibraryGame,
    XboxPlayerPlaytimes,
};
pub use profile_queries::{fetch_profile_section, XboxProfileSource};
pub use query_failure::{XboxQueryFailure, XboxQueryFailureKind, XboxSourceFailure};
pub use streaming::{
    create_scoped_stream_session, create_stream_session, create_stream_startup,
    is_stream_message_handshake_ack, stream_control_bootstrap_payloads,
    stream_control_gamepad_added_payload, stream_control_gamepad_changed_payload,
    stream_data_channel_profiles, stream_input_frame_feedback_payload,
    stream_input_metadata_bootstrap_payload, stream_message_handshake_payload,
    stream_post_handshake_payloads, XboxIceCandidate, XboxIceServer, XboxPreparedSignaling,
    XboxRemoteIceBatch, XboxStartupFailure, XboxStartupFailureKind, XboxStartupProgress,
    XboxStreamDataChannelProfile, XboxStreamFrameFeedback, XboxStreamSession, XboxStreamStartup,
    XboxStreamingAuthUpdate, XboxStreamingError, XboxWebRtcPlan,
};

const XAL_TITLE_ID: &str = "000000004c20a908";
const XAL_DEVICE_VERSION: &str = "15.0";
const XAL_CALLBACK_SCHEME: &str = "ms-xal-000000004c20a908";

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum XboxBridgeError {
    #[error("operationCancelled")]
    Cancelled,
    #[error("Xbox query failed: {failure:?}")]
    Query { failure: XboxQueryFailure },
    #[error("Retryable authentication failure: {0}")]
    RetryableAuthentication(String),
    #[error("Authentication failed: {0}")]
    Authentication(String),
    #[error("Invalid bridge data: {0}")]
    InvalidData(String),
    #[error("Profile request failed: {0}")]
    Profile(String),
    #[error("Xbox data request failed: {0}")]
    Data(String),
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct LoginStartResult {
    pub authorization_url: String,
    pub state: String,
    pub pending_json: String,
    pub seed_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AuthSession {
    pub refresh_token: String,
    pub seed_json: String,
    pub web_token_json: String,
    pub app_level: u32,
    #[uniffi(default = None)]
    pub token_cache_json: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxProfile {
    pub xuid: Option<String>,
    pub gamertag: String,
    pub display_name: String,
    pub gamer_score: String,
    pub display_picture_url: String,
    pub presence_state: Option<String>,
    pub presence_device: Option<String>,
    pub current_title_name: Option<String>,
    pub rich_presence: Option<String>,
    pub followers_count: Option<u32>,
    pub following_count: Option<u32>,
    pub friend_count: Option<u32>,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn start_login() -> Result<LoginStartResult, XboxBridgeError> {
    let output = AuthFlow::new()
        .start_oauth_login(StartOAuthLoginInput {
            title_id: XAL_TITLE_ID.to_string(),
            device_version: XAL_DEVICE_VERSION.to_string(),
        })
        .await
        .map_err(authentication_error)?;

    Ok(LoginStartResult {
        authorization_url: output.oauth_url,
        state: output.oauth_state,
        pending_json: serialize(&output.pending)?,
        seed_json: serialize(&output.seed)?,
    })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn complete_login(
    callback_url: String,
    pending_json: String,
    seed_json: String,
    force_region_ip: String,
) -> Result<AuthSession, XboxBridgeError> {
    validate_callback_url(&callback_url)?;
    let pending: PendingOAuthLogin = deserialize(&pending_json)?;
    let seed: AuthFlowSeed = deserialize(&seed_json)?;
    let force_region_ip = normalize_force_region_ip(force_region_ip);
    let output = AuthFlow::new()
        .complete_oauth_login(CompleteOAuthLoginInput {
            callback_url,
            pending,
            seed: seed.clone(),
            force_region_ip: force_region_ip.clone(),
            include_streaming_tokens: true,
        })
        .await
        .map_err(authentication_error)?;

    auth_session_from_bundle(output.auth_bundle, &seed, force_region_ip)
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn refresh_login(
    session: AuthSession,
    force_region_ip: String,
    scope: XboxAuthScope,
    force_core_refresh: bool,
    rejected_token: Option<String>,
    observer: std::sync::Arc<dyn XboxAuthObserver>,
) -> Result<AuthSession, XboxBridgeError> {
    let force_region_ip = normalize_force_region_ip(force_region_ip);
    let force_core_refresh = force_core_refresh
        || auth_session::token_was_rejected(
            &session,
            scope.flow_scope(),
            rejected_token.as_deref(),
        )?;
    Box::pin(auth_session::renew_auth_session(
        session,
        force_region_ip,
        scope.flow_scope(),
        force_core_refresh,
        observer,
    ))
    .await
}

fn authentication_error(error: AuthFlowError) -> XboxBridgeError {
    if error.requires_interactive_login() {
        XboxBridgeError::Authentication("loginCredentialsRejected".into())
    } else if let Some(upstream) = error.upstream_error() {
        query_failure::query_error(upstream.clone())
    } else {
        XboxBridgeError::InvalidData("authenticationProtocolInvalid".into())
    }
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_profile(web_token_json: String) -> Result<XboxProfile, XboxBridgeError> {
    let web_token: Value = deserialize(&web_token_json)?;
    let claims = resolve_web_token_claims(&web_token)?;
    let profile_api = ProfileApi::new(claims.uhs.clone(), claims.token.clone());
    let social_api = SocialApi::new(claims.uhs.clone(), claims.token.clone());
    let people_api = PeopleApi::new(claims.uhs.clone(), claims.token.clone());
    let presence_api = UserPresenceApi::new(claims.uhs.clone(), claims.token.clone());
    let (response, social, friend_count, presence) = tokio::join!(
        profile_api.get_current_user(),
        social_api.get_summary(),
        people_api.get_friends_count(),
        presence_api.get_current_user(),
    );
    let response = response.map_err(|error| XboxBridgeError::Profile(error.to_string()))?;
    let settings = extract_profile_settings(&response);
    let social = social.ok();
    let presence = presence.ok();

    Ok(XboxProfile {
        xuid: claims.xuid,
        gamertag: settings.get("Gamertag").cloned().unwrap_or_default(),
        display_name: settings.get("GameDisplayName").cloned().unwrap_or_default(),
        gamer_score: settings.get("Gamerscore").cloned().unwrap_or_default(),
        display_picture_url: settings
            .get("GameDisplayPicRaw")
            .cloned()
            .unwrap_or_default(),
        presence_state: presence.as_ref().and_then(|value| value.state.clone()),
        presence_device: presence.as_ref().and_then(|value| value.device.clone()),
        current_title_name: presence
            .as_ref()
            .and_then(|value| value.current_title_name.clone()),
        rich_presence: presence
            .as_ref()
            .and_then(|value| value.rich_presence.clone()),
        followers_count: social.as_ref().and_then(|value| value.followers_count),
        following_count: social.as_ref().and_then(|value| value.following_count),
        friend_count: friend_count.ok().flatten(),
    })
}

fn auth_session_from_bundle(
    bundle: xbox_auth_flow::AuthBundle,
    seed: &AuthFlowSeed,
    force_region_ip: String,
) -> Result<AuthSession, XboxBridgeError> {
    auth_session::session_from_bundle(bundle, seed, force_region_ip)
}

fn serialize<T: serde::Serialize>(value: &T) -> Result<String, XboxBridgeError> {
    serde_json::to_string(value).map_err(|error| XboxBridgeError::InvalidData(error.to_string()))
}

fn deserialize<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, XboxBridgeError> {
    serde_json::from_str(raw).map_err(|error| XboxBridgeError::InvalidData(error.to_string()))
}

pub(crate) fn normalize_force_region_ip(force_region_ip: String) -> String {
    force_region_ip.trim().to_string()
}

fn validate_callback_url(raw: &str) -> Result<(), XboxBridgeError> {
    let callback =
        url::Url::parse(raw).map_err(|error| XboxBridgeError::InvalidData(error.to_string()))?;
    if callback.scheme() != XAL_CALLBACK_SCHEME || callback.host_str() != Some("auth") {
        return Err(XboxBridgeError::InvalidData(
            "OAuth callback destination is invalid".to_string(),
        ));
    }
    Ok(())
}

pub(crate) struct WebTokenClaims {
    pub(crate) token: String,
    pub(crate) uhs: String,
    pub(crate) xuid: Option<String>,
}

pub(crate) fn resolve_web_token_claims(raw: &Value) -> Result<WebTokenClaims, XboxBridgeError> {
    let token = raw.get("data").unwrap_or(raw);
    let user_token = token
        .get("Token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| XboxBridgeError::InvalidData("web token is missing Token".to_string()))?;
    let xui = token
        .get("DisplayClaims")
        .and_then(|value| value.get("xui"))
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .ok_or_else(|| {
            XboxBridgeError::InvalidData("web token is missing DisplayClaims.xui".to_string())
        })?;
    let uhs = xui
        .get("uhs")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| XboxBridgeError::InvalidData("web token is missing uhs".to_string()))?;

    Ok(WebTokenClaims {
        token: user_token.to_string(),
        uhs: uhs.to_string(),
        xuid: xui
            .get("xid")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

fn extract_profile_settings(response: &Value) -> HashMap<String, String> {
    let root = response.get("data").unwrap_or(response);
    let settings = response
        .get("settings")
        .and_then(Value::as_array)
        .or_else(|| {
            root.get("profileUsers")
                .and_then(Value::as_array)
                .and_then(|users| users.first())
                .and_then(|user| user.get("settings"))
                .and_then(Value::as_array)
        });

    settings
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.to_string();
            let value = entry.get("value")?;
            let value = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            Some((id, value))
        })
        .collect()
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::{
        extract_profile_settings, normalize_force_region_ip, resolve_web_token_claims,
        validate_callback_url,
    };
    use serde_json::json;

    #[test]
    fn resolves_web_token_identity() {
        let claims = resolve_web_token_claims(&json!({
            "data": {
                "Token": "token",
                "DisplayClaims": {"xui": [{"uhs": "uhs", "xid": "123"}]}
            }
        }))
        .expect("claims");

        assert_eq!(claims.token, "token");
        assert_eq!(claims.uhs, "uhs");
        assert_eq!(claims.xuid.as_deref(), Some("123"));
    }

    #[test]
    fn maps_profile_settings() {
        let settings = extract_profile_settings(&json!({
            "profileUsers": [{
                "settings": [
                    {"id": "Gamertag", "value": "Player"},
                    {"id": "Gamerscore", "value": "100"}
                ]
            }]
        }));

        assert_eq!(settings.get("Gamertag").map(String::as_str), Some("Player"));
        assert_eq!(settings.get("Gamerscore").map(String::as_str), Some("100"));
    }

    #[test]
    fn validates_xal_callback_destination() {
        assert!(
            validate_callback_url("ms-xal-000000004c20a908://auth?code=code&state=state").is_ok()
        );
        assert!(validate_callback_url("https://example.com/auth?code=code").is_err());
    }

    #[test]
    fn normalizes_empty_force_region_ip() {
        assert_eq!(normalize_force_region_ip("  \n\t".to_string()), "");
    }

    #[test]
    fn normalizes_configured_force_region_ip() {
        assert_eq!(
            normalize_force_region_ip("  203.0.113.42 \n".to_string()),
            "203.0.113.42"
        );
    }
}
