use super::personal_service::{PersonalContext, PersonalService};
use crate::mods::data::types::{DataMediaSaveResult, LibraryMedia, MediaKind};
use tokio::io::AsyncWriteExt;

pub async fn resolve(
    ctx: &PersonalContext,
    kind: MediaKind,
    id: &str,
    continuation: Option<&str>,
) -> Result<LibraryMedia, String> {
    PersonalService::media(ctx, kind, continuation)
        .await?
        .items
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "资源已移除，请刷新相册".into())
}
pub async fn save(
    ctx: &PersonalContext,
    kind: MediaKind,
    id: &str,
    continuation: Option<&str>,
) -> Result<DataMediaSaveResult, String> {
    let media = resolve(ctx, kind, id, continuation).await?;
    let url = media.url.ok_or("资源地址暂不可用，请刷新相册")?;
    let parsed = reqwest::Url::parse(&url).map_err(|_| "资源地址无效")?;
    if parsed.scheme() != "https" {
        return Err("资源地址无效".into());
    }
    let extension = if kind == MediaKind::Clip {
        "mp4"
    } else {
        match parsed.path().rsplit('.').next() {
            Some("png") => "png",
            _ => "jpg",
        }
    };
    let file_name = format!(
        "Xbox-{}.{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        extension
    );
    let selected = tokio::task::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_file_name(&file_name)
            .add_filter("Xbox 媒体", &[extension])
            .save_file()
    })
    .await
    .map_err(|_| "保存窗口已中断")?;
    let Some(path) = selected else {
        return Ok(DataMediaSaveResult {
            saved: false,
            canceled: true,
        });
    };
    let temporary = path.with_file_name(format!(".xbxrc-{}.download", uuid::Uuid::new_v4()));
    let result = async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .https_only(true)
            .build()
            .map_err(|_| "下载初始化失败")?;
        let mut response = client
            .get(parsed)
            .send()
            .await
            .map_err(|_| "资源下载失败，请刷新后重试")?
            .error_for_status()
            .map_err(|_| "资源已失效，请刷新后重试")?;
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .map_err(|_| "保存路径无法写入")?;
        let mut total = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|_| "资源下载已中断")? {
            total += chunk.len() as u64;
            if total > 2 * 1024 * 1024 * 1024 {
                return Err("媒体超过 2 GB 保存上限");
            }
            file.write_all(&chunk).await.map_err(|_| "媒体写入失败")?;
        }
        file.flush().await.map_err(|_| "媒体写入失败")?;
        drop(file);
        tokio::fs::rename(&temporary, &path)
            .await
            .map_err(|_| "媒体保存失败")?;
        Ok::<_, &str>(DataMediaSaveResult {
            saved: true,
            canceled: false,
        })
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(temporary).await;
    }
    result.map_err(str::to_string)
}
