use crate::mods;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tauri::Manager;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcInvokePayload {
    pub namespace: String,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
    #[serde(rename = "diagnosticIssueId", skip_serializing_if = "Option::is_none")]
    pub diagnostic_issue_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RpcEnvelope {
    pub ok: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

impl RpcEnvelope {
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn fail(error: crate::error::AppError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(RpcError {
                diagnostic_issue_id: None,
                code: error.code().to_string(),
                message: match &error {
                    crate::error::AppError::Data(message)
                    | crate::error::AppError::DataDetailed { message, .. } => message.clone(),
                    _ => error.to_string(),
                },
                details: match &error {
                    crate::error::AppError::WebApi(error) => Some(error.safe_summary()),
                    _ => error.details().cloned(),
                },
            }),
        }
    }
}

#[tauri::command]
pub async fn rpc_invoke(payload: RpcInvokePayload, app_handle: tauri::AppHandle) -> RpcEnvelope {
    let start = Instant::now();
    let namespace = payload.namespace.clone();
    let method = payload.method.clone();
    // log::info!("[RPC][IN] {}.{}", namespace, method);

    let result = match payload.namespace.as_str() {
        "app" => mods::app_state::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "config" => mods::config::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "auth" => {
            mods::auth::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle.clone())
                .await
                .map_err(Into::into)
        }
        "data" => {
            mods::data::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle.clone())
                .await
                .map_err(|error| match error {
                    crate::error::AppError::Internal(message) => {
                        crate::error::AppError::Data(message)
                    }
                    other => other,
                })
        }
        "streaming" => mods::streaming::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "runtimeTrace" => {
            let runtime_trace = app_handle
                .state::<crate::shell::state::AppState>()
                .runtime_trace
                .clone();
            if matches!(payload.method.as_str(), "openDevTools" | "openLogDirectory") {
                diagnostic_developer_tool(&app_handle, &payload.method)
            } else if payload.method == "saveBundle" {
                let configuration = app_handle
                    .state::<crate::shell::state::AppState>()
                    .config
                    .get_by_keys(
                        &[
                            "resolution",
                            "xhome_resolution",
                            "xhome_bitrate",
                            "xcloud_bitrate",
                            "audio_bitrate",
                            "ipv6",
                            "enable_audio_control",
                            "prevent_resolution_drops",
                            "background_keepalive",
                            "diagnostics_lightweight",
                            "display_options",
                        ]
                        .map(str::to_owned),
                    )
                    .unwrap_or_default();
                mods::runtime_trace::rpc::save_bundle(payload.params, runtime_trace, configuration)
                    .await
            } else {
                mods::runtime_trace::rpc::handle_rpc(
                    payload.method.as_str(),
                    payload.params,
                    runtime_trace,
                )
                .await
                .map_err(Into::into)
            }
        }
        "system" => mods::app_state::rpc::handle_system_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "updater" => mods::updater::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        _ => Err(crate::error::AppError::Internal(format!(
            "Unknown namespace: {}",
            payload.namespace
        ))),
    };

    let duration_ms = start.elapsed().as_millis();
    match result {
        Ok(data) => {
            // log::info!(
            //     "[RPC][OUT] {}.{} ok duration={}ms",
            //     namespace,
            //     method,
            //     duration_ms
            // );
            RpcEnvelope::success(data)
        }
        Err(error) => {
            let diagnostic_issue_id = if namespace != "runtimeTrace" {
                let operation = diagnostic_rpc_operation(&namespace, &method);
                Some(
                    app_handle
                        .state::<crate::shell::state::AppState>()
                        .runtime_trace
                        .record_request_failure(
                            &operation,
                            error.code(),
                            mods::runtime_trace::service::now_ms().saturating_sub(duration_ms),
                            duration_ms,
                            diagnostic_rpc_error(&error),
                        ),
                )
            } else {
                None
            };
            log::error!(
                "[RPC][OUT] {}.{} err code={} message={} duration={}ms",
                namespace,
                method,
                error.code(),
                error.to_string(),
                duration_ms
            );
            let mut envelope = RpcEnvelope::fail(error);
            if let Some(error) = envelope.error.as_mut() {
                error.diagnostic_issue_id = diagnostic_issue_id;
            }
            envelope
        }
    }
}

fn diagnostic_developer_tool(
    app: &tauri::AppHandle,
    method: &str,
) -> crate::error::AppResult<serde_json::Value> {
    #[cfg(debug_assertions)]
    {
        use tauri_plugin_opener::OpenerExt;
        if method == "openDevTools" {
            let window = app.get_webview_window("main").ok_or_else(|| {
                crate::error::AppError::Internal("Main window unavailable".into())
            })?;
            window.open_devtools();
        } else {
            let path = mods::runtime_trace::service::trace_root_dir();
            app.opener()
                .open_path(path.to_string_lossy(), None::<&str>)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        }
        Ok(serde_json::Value::Null)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (app, method);
        Err(crate::error::AppError::InvalidParams(
            "Developer tools require a development build".into(),
        ))
    }
}

fn diagnostic_rpc_operation(namespace: &str, method: &str) -> String {
    let namespace = if matches!(
        namespace,
        "app" | "config" | "auth" | "data" | "streaming" | "system" | "updater"
    ) {
        namespace
    } else {
        "other"
    };
    const METHODS: &[&str] = &[
        "getVersion",
        "ping",
        "isFullscreen",
        "toggleFullscreen",
        "enterFullscreen",
        "exitFullscreen",
        "getStartupFlags",
        "resetAutoConnect",
        "clearUserData",
        "clearData",
        "saveBinaryFile",
        "quit",
        "restart",
        "getState",
        "checkAuthentication",
        "login",
        "clearAuthCache",
        "logout",
        "getRuntimeFeatures",
        "get",
        "set",
        "getGroups",
        "getLibraryGame",
        "getPersonalAccount",
        "getGameHistory",
        "getAchievements",
        "getPlaytimes",
        "getConsoleLibrary",
        "getMedia",
        "resolveMedia",
        "saveMedia",
        "getUserProfile",
        "getHosts",
        "getRemoteConsoles",
        "getStreamingRegions",
        "getStreamingTitleInputConfig",
        "powerOnConsole",
        "powerOffConsole",
        "sendTextToConsole",
        "getXcloudTitles",
        "refreshXcloudTitles",
        "primeXcloudTitles",
        "startSession",
        "cancelStartup",
        "getSessionProgress",
        "closeSession",
        "exchangeOffer",
        "submitIce",
        "pollIce",
        "listActiveSessions",
        "decideRecovery",
        "openExternal",
        "getChannel",
        "setChannel",
        "check",
        "downloadAndInstall",
        "relaunch",
    ];
    format!(
        "{namespace}.{}",
        if METHODS.contains(&method) {
            method
        } else {
            "other"
        }
    )
}

fn diagnostic_rpc_error(error: &crate::error::AppError) -> serde_json::Value {
    match error {
        crate::error::AppError::WebApi(error) => error.safe_summary(),
        _ => {
            let mut safe = serde_json::Map::new();
            if let Some(detail) = error.details() {
                for key in [
                    "status",
                    "statusCode",
                    "retryable",
                    "requiresLogin",
                    "durationMs",
                ] {
                    if let Some(value) = detail
                        .get(key)
                        .filter(|value| value.is_number() || value.is_boolean())
                    {
                        safe.insert(key.into(), value.clone());
                    }
                }
            }
            serde_json::Value::Object(safe)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_error_preserves_user_message_without_internal_prefix() {
        let envelope =
            RpcEnvelope::fail(crate::error::AppError::Data("查询超时，请稍后刷新".into()));
        let error = envelope.error.unwrap();
        assert_eq!(error.code, "DATA_ERROR");
        assert_eq!(error.message, "查询超时，请稍后刷新");
    }

    #[test]
    fn streaming_details_and_other_error_codes_keep_their_contract() {
        let details = serde_json::json!({"reason": "disconnected"});
        let envelope = RpcEnvelope::fail(crate::error::AppError::streaming_detailed(
            "连接中断",
            details.clone(),
        ));
        let error = envelope.error.unwrap();
        assert_eq!(error.code, "STREAMING_ERROR");
        assert_eq!(error.message, "Streaming error: 连接中断");
        assert_eq!(error.details, Some(details));
        let invalid = RpcEnvelope::fail(crate::error::AppError::InvalidParams(
            "missing parameter".into(),
        ));
        assert_eq!(invalid.error.unwrap().code, "APP_INVALID_PARAMS");
    }

    #[test]
    fn auth_recovery_rpc_exposes_safe_status_and_network_category() {
        for cause in [
            xbox_webapi::WebApiError::http(401, "secret-body"),
            xbox_webapi::WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Timeout),
        ] {
            let expected = cause.safe_summary();
            let error = RpcEnvelope::fail(crate::error::AppError::WebApi(cause))
                .error
                .unwrap();
            assert_eq!(error.code, "WEBAPI_ERROR");
            assert_eq!(error.details, Some(expected));
            assert!(!serde_json::to_string(&error.details)
                .unwrap()
                .contains("secret"));
        }
    }

    #[test]
    fn data_query_failures_preserve_safe_authentication_details_and_user_messages() {
        for cause in [
            xbox_webapi::WebApiError::auth("登录续期失效，请重新登录"),
            xbox_webapi::WebApiError::http(401, "secret-body"),
            xbox_webapi::WebApiError::http(403, "secret-body"),
            xbox_webapi::WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Timeout),
            xbox_webapi::WebApiError::parse("secret-body"),
        ] {
            let expected = cause.safe_summary();
            let failure = crate::mods::data::DataError::from(cause);
            let message = failure.to_string();
            let error = RpcEnvelope::fail(failure.into()).error.unwrap();
            assert_eq!(error.code, "DATA_ERROR");
            assert_eq!(error.message, message);
            assert_eq!(error.details, Some(expected));
            assert!(!serde_json::to_string(&error).unwrap().contains("secret"));
        }
    }
}
