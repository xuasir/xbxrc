use serde::{Deserialize, Serialize};
use xbox_touch_layouts::{ResolvedTouchLayout, TouchBundle};

#[derive(Clone, Debug, uniffi::Record)]
pub struct XboxOfficialTouchBundleChoice {
    pub id: String,
    pub tab_version: String,
    pub manifest_version: String,
    pub layout_version: String,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct XboxTouchLayoutChoice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct XboxTouchLayoutAsset {
    pub id: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct XboxTouchLayoutSnapshot {
    pub id: String,
    pub name: String,
    pub document_json: String,
    pub assets: Vec<XboxTouchLayoutAsset>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct XboxSavedTouchLayout {
    pub id: String,
    pub product_id: String,
    pub xbox_title_id: String,
    pub game_name: String,
    pub bundle_id: String,
    pub bundle_version: String,
    pub layout_id: String,
    pub layout_name: String,
    pub is_default: bool,
    pub content_hash: String,
    pub revision: String,
    pub saved_at_ms: u64,
    pub last_used_at_ms: u64,
    pub byte_count: u64,
}

#[derive(Clone)]
pub(super) struct GameIdentity {
    pub product_id: String,
    pub title_id: String,
    pub name: String,
}

pub(super) fn choice(layout: &ResolvedTouchLayout) -> XboxTouchLayoutChoice {
    XboxTouchLayoutChoice {
        id: layout.id.clone(),
        name: layout.name.clone(),
        is_default: layout.is_default,
        issues: layout.issues.clone(),
    }
}

pub(super) fn snapshot(
    bundle: &TouchBundle,
    layout: &ResolvedTouchLayout,
) -> Result<XboxTouchLayoutSnapshot, crate::XboxBridgeError> {
    let assets = layout
        .assets
        .iter()
        .map(|asset| {
            let data = bundle
                .files
                .get(&asset.path)
                .ok_or_else(|| crate::XboxBridgeError::InvalidData("布局资源缺失".into()))?;
            Ok(XboxTouchLayoutAsset {
                id: asset.id.clone(),
                data: data.clone(),
            })
        })
        .collect::<Result<_, crate::XboxBridgeError>>()?;
    Ok(XboxTouchLayoutSnapshot {
        id: layout.id.clone(),
        name: layout.name.clone(),
        document_json: serde_json::to_string(&layout.document)
            .map_err(|_| crate::XboxBridgeError::InvalidData("布局序列化失败".into()))?,
        assets,
        issues: layout.issues.clone(),
    })
}
