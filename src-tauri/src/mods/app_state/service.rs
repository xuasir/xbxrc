use crate::mods::app_state::AppStateProvider;
use crate::mods::auth::AuthProviderRef;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_store::StoreExt;
use tokio::sync::RwLock;
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
    auth_provider: AuthProviderRef,
    startup_flags: Arc<RwLock<crate::shell::state::StartupFlagsState>>,
}

#[async_trait]
impl AppStateProvider for AppStateService {
    async fn clear_user_data(&self) -> Result<ClearUserDataResult, String> {
        self.clear_renderer_storage();
        self.auth_provider
            .clear_auth_cache("ephemeral")
            .await
            .map_err(|error| error.to_string())?;
        Ok(ClearUserDataResult { cleared: true })
    }

    async fn clear_data(&self) -> Result<ClearDataResult, String> {
        self.clear_renderer_storage();

        let store = self
            .app_handle
            .store("settings.json")
            .map_err(|error| error.to_string())?;

        for key in STORE_DATA_RESET_KEYS {
            store.delete(*key);
        }
        store.save().map_err(|error| error.to_string())?;

        self.auth_provider.reset_runtime_after_store_purge().await;

        Ok(ClearDataResult {
            cleared: true,
            legacy_state_cleared: true,
        })
    }

    fn get_version(&self) -> String {
        self.app_handle.package_info().version.to_string()
    }

    fn ping(&self, message: &str) -> PingPayload {
        PingPayload {
            message: message.to_string(),
            at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn is_fullscreen(&self) -> bool {
        self.app_handle
            .get_webview_window("main")
            .map(|window| window.is_fullscreen().unwrap_or(false))
            .unwrap_or(false)
    }

    fn toggle_fullscreen(&self) -> Result<bool, String> {
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

    fn enter_fullscreen(&self) -> Result<bool, String> {
        let window = self
            .app_handle
            .get_webview_window("main")
            .ok_or("Main window is not available")?;
        window
            .set_fullscreen(true)
            .map_err(|error| error.to_string())?;
        window.is_fullscreen().map_err(|error| error.to_string())
    }

    fn exit_fullscreen(&self) -> Result<bool, String> {
        let window = self
            .app_handle
            .get_webview_window("main")
            .ok_or("Main window is not available")?;
        window
            .set_fullscreen(false)
            .map_err(|error| error.to_string())?;
        window.is_fullscreen().map_err(|error| error.to_string())
    }

    async fn get_startup_flags(&self) -> StartupFlagsPayload {
        let flags = self.startup_flags.read().await;

        StartupFlagsPayload {
            fullscreen: flags.fullscreen,
            auto_connect: flags.auto_connect.clone(),
        }
    }

    async fn reset_auto_connect(&self) -> bool {
        let mut flags = self.startup_flags.write().await;
        flags.auto_connect.clear();
        true
    }

    async fn quit(&self) {
        crate::shell::terminate(&self.app_handle).await;
        self.app_handle.exit(0);
    }

    async fn restart(&self) {
        self.restart_delayed(10).await;
    }

    // 与 Electron 保持一致：先返回 RPC，再异步触发重启，避免响应被截断。
    async fn restart_delayed(&self, delay_ms: u64) {
        crate::shell::terminate(&self.app_handle).await;
        let app_handle = self.app_handle.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            app_handle.restart();
        });
    }

    fn open_external(&self, url: &str) -> Result<(), String> {
        self.app_handle
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|error| error.to_string())
    }
}

impl AppStateService {
    pub fn new(
        app_handle: AppHandle,
        auth_provider: AuthProviderRef,
        startup_flags: Arc<RwLock<crate::shell::state::StartupFlagsState>>,
    ) -> Self {
        Self {
            app_handle,
            auth_provider,
            startup_flags,
        }
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
}
