use crate::error::AppResult;
use crate::mods::auth::service::AUTH_WINDOW_LABEL;
use crate::{mods::auth::events, AppState};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const XAL_REDIRECT_URI_PREFIX: &str = "ms-xal-000000004c20a908://auth";

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum AuthCommand {
    GetState,
    Login,
    CheckAuthentication,
    ClearAuthCache { scope: Option<String> },
    Logout,
    HandleOAuthCallback { url: String },
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: tauri::AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<AppState>();
    let auth = state.auth.clone();

    // 转换到强类型命令
    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: AuthCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid auth command params: {}", e))
    })?;

    match command {
        AuthCommand::GetState => Ok(serde_json::to_value(auth.get_state())?),
        AuthCommand::Login => {
            let login_result = auth.login().await?;
            let auth_state = auth.get_state();

            if login_result.mode == "oauth-window" {
                open_oauth_window(&app_handle, &login_result.url)?;

                return Ok(json!({
                    "provider": auth_state.provider,
                    "mode": "oauth-window",
                    "oauth": {
                        "url": login_result.url,
                        "state": login_result.state
                    }
                }));
            }

            Ok(json!({
                "provider": auth_state.provider,
                "mode": login_result.mode
            }))
        }
        AuthCommand::CheckAuthentication => {
            let result = auth.check_authentication().await?;
            let auth_state = auth.get_state();

            if auth_state.is_authenticated {
                let _ = events::emit_session_ready(
                    &app_handle,
                    &auth_state.provider,
                    auth_state.app_level,
                );
            }

            Ok(serde_json::to_value(result)?)
        }
        AuthCommand::ClearAuthCache { scope } => {
            let scope = scope.unwrap_or_else(|| "ephemeral".to_string());
            auth.clear_auth_cache(&scope).await?;

            Ok(json!({
                "cleared": true,
                "scope": scope
            }))
        }
        AuthCommand::Logout => {
            auth.logout().await?;
            Ok(json!({ "loggedOut": true }))
        }
        AuthCommand::HandleOAuthCallback { url } => {
            auth.handle_oauth_callback(&url).await?;
            let auth_state = auth.get_state();

            if auth_state.is_authenticated {
                let _ = events::emit_session_ready(
                    &app_handle,
                    &auth_state.provider,
                    auth_state.app_level,
                );
            }

            Ok(json!({ "success": true }))
        }
    }
}

fn open_oauth_window(app_handle: &tauri::AppHandle, oauth_url: &str) -> AppResult<()> {
    if oauth_url.trim().is_empty() {
        return Err(crate::error::AppError::Data(
            "Missing OAuth URL".to_string(),
        ));
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
            let auth = state.auth.clone();
            let callback_result = auth.handle_oauth_callback(&callback_url).await;
            let auth_state = auth.get_state();

            if callback_result.is_ok() && auth_state.is_authenticated {
                eprintln!("[auth][window] callback handled and authenticated");
                let _ = events::emit_session_ready(
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
                let auth = state.auth.clone();
                auth.cancel_pending_login().await;
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
