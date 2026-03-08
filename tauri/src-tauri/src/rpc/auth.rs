use crate::mods::auth::service::AUTH_WINDOW_LABEL;
use crate::{event_bridge, AppState};
use serde_json::{json, Value};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const XAL_REDIRECT_URI_PREFIX: &str = "ms-xal-000000004c20a908://auth";

pub async fn handle_auth_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<AppState>();

    match method {
        "getState" => {
            let auth = state.auth.read().await;
            Ok(serde_json::to_value(auth.get_state()).map_err(|e| e.to_string())?)
        }
        "login" => {
            let mut auth = state.auth.write().await;
            let login_result = auth.login().await?;
            let auth_state = auth.get_state();
            drop(auth);

            let mode = login_result
                .get("mode")
                .and_then(|value| value.as_str())
                .unwrap_or("oauth-window");

            if mode == "oauth-window" {
                let oauth_url = login_result
                    .get("url")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                open_oauth_window(&app_handle, oauth_url)?;

                return Ok(json!({
                    "provider": auth_state.provider,
                    "mode": "oauth-window",
                    "oauth": {
                        "url": oauth_url,
                        "state": login_result.get("state").and_then(|value| value.as_str()).unwrap_or_default()
                    }
                }));
            }

            Ok(json!({
                "provider": auth_state.provider,
                "mode": mode
            }))
        }
        "checkAuthentication" => {
            let mut auth = state.auth.write().await;
            let result = auth.check_authentication().await?;
            let auth_state = auth.get_state();
            drop(auth);

            if auth_state.is_authenticated {
                let _ = event_bridge::emit_auth_session_ready(
                    &app_handle,
                    &auth_state.provider,
                    auth_state.app_level,
                );
            }

            Ok(result)
        }
        "clearAuthCache" => {
            let scope = params
                .as_ref()
                .and_then(|payload| payload.get("scope"))
                .and_then(|value| value.as_str())
                .unwrap_or("ephemeral");

            let mut auth = state.auth.write().await;
            auth.clear_auth_cache(scope)?;

            Ok(json!({
                "cleared": true,
                "scope": scope
            }))
        }
        "logout" => {
            let mut auth = state.auth.write().await;
            auth.logout()?;
            Ok(json!({ "loggedOut": true }))
        }
        // 非 shared contract 方法，保留给 OAuth 回调桥接使用。
        "handleOAuthCallback" => {
            let callback_url = params
                .as_ref()
                .and_then(|payload| payload.get("url"))
                .and_then(|value| value.as_str())
                .ok_or("Missing url parameter")?;

            let mut auth = state.auth.write().await;
            auth.handle_oauth_callback(callback_url).await?;
            let auth_state = auth.get_state();
            drop(auth);

            if auth_state.is_authenticated {
                let _ = event_bridge::emit_auth_session_ready(
                    &app_handle,
                    &auth_state.provider,
                    auth_state.app_level,
                );
            }

            Ok(serde_json::to_value(auth_state).map_err(|e| e.to_string())?)
        }
        _ => Err(format!("Unknown method in auth: {}", method)),
    }
}

fn open_oauth_window(app_handle: &tauri::AppHandle, oauth_url: &str) -> Result<(), String> {
    if oauth_url.trim().is_empty() {
        return Err("Missing OAuth URL".to_string());
    }

    if let Some(existing) = app_handle.get_webview_window(AUTH_WINDOW_LABEL) {
        eprintln!("[auth][window] close existing auth window");
        let _ = existing.close();
    }

    let external_url = url::Url::parse(oauth_url).map_err(|error| error.to_string())?;
    let app_handle_for_nav = app_handle.clone();

    let auth_window = WebviewWindowBuilder::new(
        app_handle,
        AUTH_WINDOW_LABEL,
        WebviewUrl::External(external_url),
    )
    .title("Authentication")
    .inner_size(540.0, 740.0)
    .resizable(true)
    .center()
    .on_navigation(move |target_url| {
        if !is_oauth_callback_target(target_url) {
            return true;
        }
        eprintln!(
            "[auth][window] capture callback target={}",
            target_url.as_str()
        );

        let callback_url = target_url.as_str().to_string();
        let app_handle = app_handle_for_nav.clone();
        tauri::async_runtime::spawn(async move {
            let state = app_handle.state::<AppState>();
            let mut auth = state.auth.write().await;
            let callback_result = auth.handle_oauth_callback(&callback_url).await;
            let auth_state = auth.get_state();
            drop(auth);

            if callback_result.is_ok() && auth_state.is_authenticated {
                eprintln!("[auth][window] callback handled and authenticated");
                let _ = event_bridge::emit_auth_session_ready(
                    &app_handle,
                    &auth_state.provider,
                    auth_state.app_level,
                );
            }

            if let Some(window) = app_handle.get_webview_window(AUTH_WINDOW_LABEL) {
                eprintln!("[auth][window] close auth window after callback");
                let _ = window.close();
            }
        });
        false
    })
    .build()
    .map_err(|error| error.to_string())?;

    let app_handle_for_close = app_handle.clone();
    auth_window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
        ) {
            eprintln!("[auth][window] user closed auth window");
            let app_handle = app_handle_for_close.clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<AppState>();
                let mut auth = state.auth.write().await;
                auth.cancel_pending_login();
            });
        }
    });

    let _ = auth_window.show();
    let _ = auth_window.set_focus();
    eprintln!("[auth][window] opened oauth window");
    Ok(())
}

fn is_oauth_callback_target(target_url: &url::Url) -> bool {
    if target_url.as_str().starts_with(XAL_REDIRECT_URI_PREFIX) {
        return true;
    }

    let mut has_code = false;
    let mut has_state = false;
    for (key, value) in target_url.query_pairs() {
        if key == "code" && !value.is_empty() {
            has_code = true;
        } else if key == "state" && !value.is_empty() {
            has_state = true;
        }
    }

    has_code && has_state
}
