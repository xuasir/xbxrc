mod community_service;
mod storage_repository;
mod types;

pub use community_service::{fetch_community_touch_bundle, fetch_community_touch_bundles};

use crate::{cloud_access::load_scoped_stream_access, XboxBridgeError};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
pub use types::{
    XboxOfficialTouchBundleChoice, XboxSavedTouchLayout, XboxTouchLayoutAsset,
    XboxTouchLayoutChoice, XboxTouchLayoutSnapshot,
};
use xbox_streaming::Target;
use xbox_touch_layouts::{decode_bundle, title_bundles, TouchBundle, TouchBundleDescriptor};
use xbox_webapi::XcloudApi;

#[derive(uniffi::Object)]
pub struct XboxOfficialTouchBundle {
    identity: types::GameIdentity,
    bundle_id: String,
    original: Vec<u8>,
    decoded: TouchBundle,
}

#[uniffi::export(async_runtime = "tokio")]
impl XboxOfficialTouchBundle {
    pub fn choices(&self) -> Vec<XboxTouchLayoutChoice> {
        self.decoded.layouts.iter().map(types::choice).collect()
    }
    pub fn version(&self) -> String {
        self.decoded.version.clone()
    }

    pub async fn preview(
        &self,
        layout_id: String,
    ) -> Result<XboxTouchLayoutSnapshot, XboxBridgeError> {
        let layout = self
            .decoded
            .layouts
            .iter()
            .find(|layout| layout.id == layout_id)
            .ok_or_else(|| XboxBridgeError::InvalidData("布局身份无效".into()))?;
        types::snapshot(&self.decoded, layout)
    }
}

#[derive(uniffi::Object)]
pub struct XboxTouchLayoutLibrary {
    root: PathBuf,
    operations: Arc<Mutex<()>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl XboxTouchLayoutLibrary {
    #[uniffi::constructor]
    pub fn new(root_directory: String) -> Result<Arc<Self>, XboxBridgeError> {
        let root = PathBuf::from(root_directory);
        if !root.is_absolute() {
            return Err(XboxBridgeError::InvalidData(
                "布局库需要应用内绝对路径".into(),
            ));
        }
        Ok(Arc::new(Self {
            root,
            operations: Arc::new(Mutex::new(())),
        }))
    }

    pub async fn saved_layouts(&self) -> Result<Vec<XboxSavedTouchLayout>, XboxBridgeError> {
        let root = self.root.clone();
        let operations = self.operations.clone();
        blocking(move || {
            let _guard = operations.lock().map_err(|_| invalid("布局库操作中断"))?;
            let entries = storage_repository::read_index(&root)?;
            storage_repository::collect_unused(&root, &entries)?;
            Ok(entries)
        })
        .await
    }

    pub async fn save(
        &self,
        bundle: Arc<XboxOfficialTouchBundle>,
        layout_id: String,
    ) -> Result<Vec<XboxSavedTouchLayout>, XboxBridgeError> {
        let root = self.root.clone();
        let operations = self.operations.clone();
        blocking(move || {
            let _guard = operations.lock().map_err(|_| invalid("布局库操作中断"))?;
            storage_repository::save(
                &root,
                &bundle.identity,
                &bundle.bundle_id,
                &bundle.original,
                &bundle.decoded,
                &layout_id,
            )
        })
        .await
    }

    pub async fn load_layout(
        &self,
        id: String,
    ) -> Result<XboxTouchLayoutSnapshot, XboxBridgeError> {
        let root = self.root.clone();
        let operations = self.operations.clone();
        blocking(move || {
            let _guard = operations.lock().map_err(|_| invalid("布局库操作中断"))?;
            storage_repository::load(&root, &id)
        })
        .await
    }

    pub async fn delete_layout(
        &self,
        id: String,
    ) -> Result<Vec<XboxSavedTouchLayout>, XboxBridgeError> {
        let root = self.root.clone();
        let operations = self.operations.clone();
        blocking(move || {
            let _guard = operations.lock().map_err(|_| invalid("布局库操作中断"))?;
            storage_repository::delete(&root, &id)
        })
        .await
    }

    pub async fn mark_used(&self, id: String) -> Result<(), XboxBridgeError> {
        let root = self.root.clone();
        let operations = self.operations.clone();
        blocking(move || {
            let _guard = operations.lock().map_err(|_| invalid("布局库操作中断"))?;
            storage_repository::mark_used(&root, &id)
        })
        .await
    }
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_official_touch_bundles(
    access_handle: String,
    xbox_title_id: String,
    product_id: String,
) -> Result<Vec<XboxOfficialTouchBundleChoice>, XboxBridgeError> {
    let access = load_scoped_stream_access(&access_handle, Target::Cloud, None, None)?;
    let title_id = normalize_title(&xbox_title_id)?;
    let product_id = normalize_product(&product_id)?;
    let api = crate::cloud_access::query_xcloud_api(&access_handle, &access, None)?;
    let bundles = cloud_title_bundles(&api, &product_id, &title_id).await?;
    load_scoped_stream_access(
        &access_handle,
        Target::Cloud,
        Some(&access.account_id),
        Some(access.owner_generation),
    )?;
    Ok(bundles
        .into_iter()
        .map(|bundle| XboxOfficialTouchBundleChoice {
            id: bundle.id,
            tab_version: bundle.tab_version,
            manifest_version: bundle.manifest_version,
            layout_version: bundle.layout_version,
        })
        .collect())
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_official_touch_bundle(
    access_handle: String,
    xbox_title_id: String,
    bundle_id: String,
    product_id: String,
    game_name: String,
    locale: String,
    display_scale: f64,
) -> Result<Arc<XboxOfficialTouchBundle>, XboxBridgeError> {
    let access = load_scoped_stream_access(&access_handle, Target::Cloud, None, None)?;
    let title_id = normalize_title(&xbox_title_id)?;
    let product_id = normalize_product(&product_id)?;
    let api = crate::cloud_access::query_xcloud_api(&access_handle, &access, None)?;
    let bundles = cloud_title_bundles(&api, &product_id, &title_id).await?;
    if !bundles.iter().any(|bundle| bundle.id == bundle_id) {
        return Err(invalid("此官方布局包未关联当前游戏或版本等待支持"));
    }
    let original = api
        .touch_adaptation_bundle(&bundle_id)
        .await
        .map_err(crate::query_failure::query_error)?;
    load_scoped_stream_access(
        &access_handle,
        Target::Cloud,
        Some(&access.account_id),
        Some(access.owner_generation),
    )?;
    let identity = types::GameIdentity {
        product_id,
        title_id,
        name: game_name.chars().take(120).collect(),
    };
    blocking(move || {
        let decoded = decode_bundle(&original, &locale, display_scale)
            .map_err(|error| invalid(&error.to_string()))?;
        Ok(Arc::new(XboxOfficialTouchBundle {
            identity,
            bundle_id,
            original,
            decoded,
        }))
    })
    .await
}

async fn cloud_title_bundles(
    api: &XcloudApi,
    product_id: &str,
    title_id: &str,
) -> Result<Vec<TouchBundleDescriptor>, XboxBridgeError> {
    // 云游戏布局随标题详情下发；目录与下载归属校验共用此查询。
    let response = api
        .get_titles_by_ids(&[product_id.to_owned()], "productId")
        .await
        .map_err(crate::query_failure::query_error)?;
    title_bundles(&response, product_id, title_id).map_err(|error| invalid(&error.to_string()))
}

fn normalize_product(product: &str) -> Result<String, XboxBridgeError> {
    if product.is_empty()
        || product.len() > 32
        || !product.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(invalid("游戏商品身份无效"));
    }
    Ok(product.to_ascii_uppercase())
}

fn normalize_title(title: &str) -> Result<String, XboxBridgeError> {
    title
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.to_string())
        .ok_or_else(|| invalid("Xbox 游戏身份无效"))
}

fn invalid(reason: &str) -> XboxBridgeError {
    XboxBridgeError::InvalidData(reason.into())
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, XboxBridgeError> + Send + 'static,
) -> Result<T, XboxBridgeError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| invalid("布局资源处理中断"))?
}

#[cfg(test)]
#[path = "storage.test.rs"]
mod tests;
