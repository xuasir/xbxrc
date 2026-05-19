use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, ResourceId};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

use crate::event_bridge;
use crate::settings_store::SettingsStoreResolver;

use super::channel::UpdateChannel;
use super::endpoints;
use super::events::UPDATER_PROGRESS_CHANNEL;

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
    pending_update_rid: Arc<Mutex<Option<ResourceId>>>,
}

impl UpdaterService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            pending_update_rid: Arc::new(Mutex::new(None)),
        }
    }

    fn clear_pending_update(&self) {
        let mut pending = self
            .pending_update_rid
            .lock()
            .expect("updater pending_update_rid mutex poisoned");
        if let Some(rid) = pending.take() {
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
        let channel = self.get_channel()?;
        let endpoint = endpoints::endpoint_for(channel);
        let current_version = self.app_handle.package_info().version.to_string();

        self.clear_pending_update();

        let endpoint_url = Url::parse(&endpoint).map_err(|error| error.to_string())?;
        let updater = self
            .app_handle
            .updater_builder()
            .endpoints(vec![endpoint_url])
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())?;

        let update = updater.check().await.map_err(|error| error.to_string())?;

        if let Some(update) = update {
            let result = UpdaterCheckResult {
                current_version,
                update_available: true,
                version: Some(update.version.clone()),
                notes: update.body.clone(),
                date: update.date.map(|value| value.to_string()),
            };
            let rid = self.app_handle.resources_table().add(update);
            let mut pending = self
                .pending_update_rid
                .lock()
                .expect("updater pending_update_rid mutex poisoned");
            *pending = Some(rid);
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
        let rid = {
            let mut pending = self
                .pending_update_rid
                .lock()
                .expect("updater pending_update_rid mutex poisoned");
            pending.take().ok_or_else(|| {
                "No pending update. Call updater.check before downloadAndInstall.".to_string()
            })?
        };

        let update = self
            .app_handle
            .resources_table()
            .get::<tauri_plugin_updater::Update>(rid)
            .map_err(|error| error.to_string())?;
        let update = (*update).clone();

        let app_handle = self.app_handle.clone();
        let mut started = false;
        let mut downloaded_total = 0usize;

        update
            .download_and_install(
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
                || {
                    let _ = event_bridge::emit(
                        &app_handle,
                        UPDATER_PROGRESS_CHANNEL,
                        &UpdaterProgressEvent::Finished,
                    );
                },
            )
            .await
            .map_err(|error| error.to_string())?;

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
