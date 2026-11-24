use super::{
    blocking, invalid, normalize_product, normalize_title, types, XboxOfficialTouchBundle,
    XboxOfficialTouchBundleChoice,
};
use crate::{query_failure::query_error, XboxBridgeError};
use std::{sync::Arc, time::Duration};
use xbox_touch_layouts::{community_bundle, validate_community_game};
use xbox_webapi::CommunityTouchApi;

const PREFIX: &str = "community:better-xcloud:";

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_community_touch_bundles(
    xbox_title_id: String,
    product_id: String,
) -> Result<Vec<XboxOfficialTouchBundleChoice>, XboxBridgeError> {
    let title_id = normalize_title(&xbox_title_id)?;
    let product_id = normalize_product(&product_id)?;
    tokio::time::timeout(Duration::from_secs(30), async {
        let api = CommunityTouchApi::default();
        let revision = api.latest_revision().await.map_err(query_error)?;
        let Some(definition) = api
            .definition(&revision, &title_id)
            .await
            .map_err(query_error)?
        else {
            return Ok(Vec::new());
        };
        validate_community_game(&definition, &product_id)
            .map_err(|error| invalid(&error.to_string()))?;
        Ok(vec![XboxOfficialTouchBundleChoice {
            id: format!("{PREFIX}{revision}"),
            tab_version: revision[..12].to_owned(),
            manifest_version: "1.0".into(),
            layout_version: "4.1".into(),
        }])
    })
    .await
    .map_err(|_| invalid("社区布局查询超时，请重试"))?
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_community_touch_bundle(
    xbox_title_id: String,
    bundle_id: String,
    product_id: String,
    game_name: String,
    locale: String,
    display_scale: f64,
) -> Result<Arc<XboxOfficialTouchBundle>, XboxBridgeError> {
    let title_id = normalize_title(&xbox_title_id)?;
    let product_id = normalize_product(&product_id)?;
    let revision = bundle_id
        .strip_prefix(PREFIX)
        .filter(|value| {
            value.len() == 40
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| invalid("社区布局来源身份无效"))?
        .to_owned();
    let (definition, sources) = tokio::time::timeout(Duration::from_secs(30), async {
        let api = CommunityTouchApi::default();
        let definition = api
            .definition(&revision, &title_id)
            .await
            .map_err(query_error)?
            .ok_or_else(|| invalid("此游戏暂无社区触屏布局"))?;
        validate_community_game(&definition, &product_id)
            .map_err(|error| invalid(&error.to_string()))?;
        let sources = api
            .layouts(&revision, &definition)
            .await
            .map_err(query_error)?;
        Ok::<_, XboxBridgeError>((definition, sources))
    })
    .await
    .map_err(|_| invalid("社区布局下载超时，请重试"))??;
    blocking(move || {
        let (original, decoded) = community_bundle(
            &definition,
            &sources,
            &revision,
            &product_id,
            &locale,
            display_scale,
        )
        .map_err(|error| invalid(&error.to_string()))?;
        Ok(Arc::new(XboxOfficialTouchBundle {
            identity: types::GameIdentity {
                product_id,
                title_id,
                name: game_name.chars().take(120).collect(),
            },
            bundle_id,
            original,
            decoded,
        }))
    })
    .await
}
