use crate::AppState;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tauri_plugin_keepawake::TauriPluginKeepawakeExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_store::StoreExt;
use tokio::time::Duration;

const STORE_DATA_RESET_KEYS: &[&str] = &[
    // 与 Electron clearData 对齐：清 auth token + data cache，不清配置。
    "auth.tokens.core",
    "auth.tokens.stream",
    "auth.tokens.web",
    "data.profile",
    "data.profileCache",
    "data.xcloud.titles",
    "data.xcloudTitlesCache",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearUserDataResult {
    pub cleared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearDataResult {
    pub cleared: bool,
    pub legacy_state_cleared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupFlagsPayload {
    pub fullscreen: bool,
    pub auto_connect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingPayload {
    pub message: String,
    pub at: String,
}

pub struct AppStateService {
    app_handle: AppHandle,
}

impl AppStateService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    // 清 session/ephemeral token，不触及主登录 token。
    pub async fn clear_user_data(&self) -> Result<ClearUserDataResult, String> {
        self.clear_renderer_storage();

        let state = self.app_handle.state::<AppState>();
        let mut auth = state.auth.write().await;
        auth.clear_auth_cache("ephemeral")?;

        Ok(ClearUserDataResult { cleared: true })
    }

    // 全量清理（不包含 config.settings），并重置 auth runtime。
    pub async fn clear_data(&self) -> Result<ClearDataResult, String> {
        self.clear_renderer_storage();

        let store = self
            .app_handle
            .store("settings.json")
            .map_err(|error| error.to_string())?;

        for key in STORE_DATA_RESET_KEYS {
            store.delete(*key);
        }
        store.save().map_err(|error| error.to_string())?;

        let state = self.app_handle.state::<AppState>();
        let mut auth = state.auth.write().await;
        auth.reset_runtime_after_store_purge();

        Ok(ClearDataResult {
            cleared: true,
            legacy_state_cleared: true,
        })
    }

    pub fn get_version(&self) -> String {
        self.app_handle.package_info().version.to_string()
    }

    pub fn ping(&self, message: &str) -> PingPayload {
        PingPayload {
            message: message.to_string(),
            at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        self.app_handle
            .get_webview_window("main")
            .map(|window| window.is_fullscreen().unwrap_or(false))
            .unwrap_or(false)
    }

    pub fn toggle_fullscreen(&self) -> Result<bool, String> {
        let window = self
            .app_handle
            .get_webview_window("main")
            .ok_or("Main window is not available")?;
        let next = !window.is_fullscreen().map_err(|error| error.to_string())?;
        window
            .set_fullscreen(next)
            .map_err(|error| error.to_string())?;
        window.is_fullscreen().map_err(|error| error.to_string())
    }

    pub fn enter_fullscreen(&self) -> Result<bool, String> {
        let window = self
            .app_handle
            .get_webview_window("main")
            .ok_or("Main window is not available")?;
        window
            .set_fullscreen(true)
            .map_err(|error| error.to_string())?;
        window.is_fullscreen().map_err(|error| error.to_string())
    }

    pub fn exit_fullscreen(&self) -> Result<bool, String> {
        let window = self
            .app_handle
            .get_webview_window("main")
            .ok_or("Main window is not available")?;
        window
            .set_fullscreen(false)
            .map_err(|error| error.to_string())?;
        window.is_fullscreen().map_err(|error| error.to_string())
    }

    pub async fn get_startup_flags(&self) -> StartupFlagsPayload {
        let app_state = self.app_handle.state::<AppState>();
        let flags = app_state.startup_flags.read().await;

        StartupFlagsPayload {
            fullscreen: flags.fullscreen,
            auto_connect: flags.auto_connect.clone(),
        }
    }

    pub async fn reset_auto_connect(&self) -> bool {
        let app_state = self.app_handle.state::<AppState>();
        let mut flags = app_state.startup_flags.write().await;
        flags.auto_connect.clear();
        true
    }

    pub async fn quit(&self) {
        self.prepare_runtime_shutdown().await;
        let app_state = self.app_handle.state::<AppState>();
        app_state.is_quitting.store(true, Ordering::Relaxed);
        self.stop_prevent_display_sleep();
        self.app_handle.exit(0);
    }

    pub async fn restart(&self) {
        self.restart_delayed(10).await;
    }

    // 与 Electron 保持一致：先返回 RPC，再异步触发重启，避免响应被截断。
    pub async fn restart_delayed(&self, delay_ms: u64) {
        self.prepare_runtime_shutdown().await;
        let app_state = self.app_handle.state::<AppState>();
        app_state.is_quitting.store(true, Ordering::Relaxed);
        self.stop_prevent_display_sleep();
        let app_handle = self.app_handle.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            app_handle.restart();
        });
    }

    pub fn open_external(&self, url: &str) -> Result<(), String> {
        self.app_handle
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|error| error.to_string())
    }

    fn clear_renderer_storage(&self) {
        let Some(window) = self.app_handle.get_webview_window("main") else {
            return;
        };

        // 仅做 best-effort：失败不阻断主流程。
        let script = r#"
            (() => {
              try { window.localStorage?.clear(); } catch (_) {}
              try { window.sessionStorage?.clear(); } catch (_) {}
              try {
                const cookies = document.cookie?.split(';') ?? [];
                for (const item of cookies) {
                  const key = item.split('=')[0]?.trim();
                  if (!key) continue;
                  document.cookie = `${key}=;expires=Thu, 01 Jan 1970 00:00:00 GMT;path=/`;
                }
              } catch (_) {}
              try {
                if (window.caches?.keys) {
                  window.caches.keys().then((keys) => {
                    keys.forEach((key) => {
                      try { window.caches.delete(key); } catch (_) {}
                    });
                  });
                }
              } catch (_) {}
              try {
                if (navigator?.serviceWorker?.getRegistrations) {
                  navigator.serviceWorker.getRegistrations().then((regs) => {
                    regs.forEach((reg) => {
                      try { reg.unregister(); } catch (_) {}
                    });
                  });
                }
              } catch (_) {}
              try {
                if (window.indexedDB?.databases) {
                  window.indexedDB.databases().then((dbs) => {
                    dbs?.forEach((db) => {
                      if (db && db.name) {
                        try { window.indexedDB.deleteDatabase(db.name); } catch (_) {}
                      }
                    });
                  });
                }
              } catch (_) {}
            })();
        "#;

        let _ = window.eval(script);
    }

    fn stop_prevent_display_sleep(&self) {
        // 退出路径只做 best-effort，避免 stop 失败阻断退出。
        let _ = self
            .app_handle
            .tauri_plugin_keepawake()
            .stop(&self.app_handle);
    }

    async fn prepare_runtime_shutdown(&self) {
        let state = self.app_handle.state::<AppState>();
        state.gamepad.shutdown();
        state.xbxengine.shutdown().await;
        state.streaming.shutdown().await;
    }
}
