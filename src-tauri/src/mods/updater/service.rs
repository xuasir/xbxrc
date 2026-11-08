use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

use crate::event_bridge;
use crate::settings_store::SettingsStoreResolver;

use super::channel::UpdateChannel;
use super::endpoints;
use super::events::UPDATER_PROGRESS_CHANNEL;
use super::runtime_state::UpdaterRuntimeState;

const UPDATE_CHANNEL_STORE_KEY: &str = "update_channel";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "event")]
pub enum UpdaterProgressEvent {
    Started {
        content_length: Option<u64>,
    },
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    Finished,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterCheckResult {
    pub current_version: String,
    pub update_available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

pub struct UpdaterService {
    app_handle: AppHandle,
    runtime: Arc<UpdaterRuntimeState>,
}

impl UpdaterService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            runtime: Arc::new(UpdaterRuntimeState::default()),
        }
    }

    fn clear_pending_update(&self) {
        let (_, rid) = self.runtime.invalidate();
        if let Some(rid) = rid {
            let _ = self.app_handle.resources_table().close(rid);
        }
    }

    pub fn get_channel(&self) -> Result<UpdateChannel, String> {
        let store = SettingsStoreResolver::new(self.app_handle.clone()).open_read()?;
        let value = store.store().get(UPDATE_CHANNEL_STORE_KEY);
        match value {
            Some(value) => {
                let channel = value
                    .as_str()
                    .ok_or_else(|| "update_channel must be a string".to_string())?;
                UpdateChannel::parse(channel)
                    .ok_or_else(|| format!("Unknown update channel: {channel}"))
            }
            None => Ok(UpdateChannel::default_channel()),
        }
    }

    pub fn set_channel(&self, channel: UpdateChannel) -> Result<(), String> {
        let store = SettingsStoreResolver::new(self.app_handle.clone()).open_write()?;
        store
            .store()
            .set(UPDATE_CHANNEL_STORE_KEY, json!(channel.as_str()));
        store.save()?;
        self.clear_pending_update();
        Ok(())
    }

    pub async fn check(&self) -> Result<UpdaterCheckResult, String> {
        let (generation, previous) = self.runtime.invalidate();
        if let Some(rid) = previous {
            let _ = self.app_handle.resources_table().close(rid);
        }
        let channel = self.get_channel()?;
        let endpoint = endpoints::endpoint_for(channel);
        let current_version = self.app_handle.package_info().version.to_string();

        let endpoint_url = Url::parse(&endpoint).map_err(|error| error.to_string())?;
        let updater = self
            .app_handle
            .updater_builder()
            .timeout(Duration::from_secs(20))
            .endpoints(vec![endpoint_url])
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())?;

        let update = updater
            .check()
            .await
            .map_err(|_| "更新检查失败，请检查网络后重试".to_string())?;
        if !self.runtime.is_current(generation) {
            return Err("更新检查结果已改变，请重试".into());
        }

        if let Some(mut update) = update {
            update.timeout = Some(Duration::from_secs(300));
            super::manifest::validate_update_manifest(
                &update.raw_json,
                channel.as_str(),
                env!("XBX_BUILD_EDITION"),
                env!("XBX_APP_IDENTIFIER"),
            )?;
            let result = UpdaterCheckResult {
                current_version,
                update_available: true,
                version: Some(update.version.clone()),
                notes: update.body.clone(),
                date: update.date.map(|value| value.to_string()),
            };
            let rid = self.app_handle.resources_table().add(update);
            if !self.runtime.publish(generation, rid) {
                let _ = self.app_handle.resources_table().close(rid);
                return Err("更新检查结果已改变，请重试".into());
            }
            Ok(result)
        } else {
            Ok(UpdaterCheckResult {
                current_version,
                update_available: false,
                version: None,
                notes: None,
                date: None,
            })
        }
    }

    pub async fn download_and_install(&self) -> Result<(), String> {
        let lease = self.runtime.begin_download()?;
        let rid = lease.rid;

        let update = self
            .app_handle
            .resources_table()
            .get::<tauri_plugin_updater::Update>(rid)
            .map_err(|error| error.to_string())?;
        let update = (*update).clone();

        let app_handle = self.app_handle.clone();
        let mut started = false;
        let mut downloaded_total = 0usize;

        let bytes = update
            .download(
                |chunk_length, content_length| {
                    if !started {
                        started = true;
                        downloaded_total = 0;
                        let _ = event_bridge::emit(
                            &app_handle,
                            UPDATER_PROGRESS_CHANNEL,
                            &UpdaterProgressEvent::Started { content_length },
                        );
                    }
                    downloaded_total += chunk_length;
                    let _ = event_bridge::emit(
                        &app_handle,
                        UPDATER_PROGRESS_CHANNEL,
                        &UpdaterProgressEvent::Progress {
                            downloaded: downloaded_total as u64,
                            content_length,
                        },
                    );
                },
                || {},
            )
            .await
            .map_err(|_| "更新下载或签名校验失败，请重试".to_string())?;
        lease.install_if_current(|| {
            update
                .install(bytes)
                .map_err(|_| "更新安装失败，请重试".to_string())
        })?;
        lease.complete();
        let _ = event_bridge::emit(
            &app_handle,
            UPDATER_PROGRESS_CHANNEL,
            &UpdaterProgressEvent::Finished,
        );

        let _ = self.app_handle.resources_table().close(rid);
        Ok(())
    }

    pub fn relaunch(&self) -> Result<(), String> {
        self.app_handle.request_restart();
        Ok(())
    }
}

pub type UpdaterServiceRef = Arc<UpdaterService>;

pub fn channel_to_json(channel: UpdateChannel) -> Value {
    json!({ "channel": channel.as_str() })
}
