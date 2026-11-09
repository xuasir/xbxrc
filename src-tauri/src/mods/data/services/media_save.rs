use super::personal_service::{PersonalContext, PersonalService};
use crate::mods::data::session_resolver::DataSessionResolver;
use crate::mods::data::types::{DataMediaSaveResult, LibraryMedia, MediaKind};
use crate::mods::data::DataError;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use xbox_webapi::WebApiError;

const MAX_MEDIA_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub async fn resolve(
    ctx: &PersonalContext,
    kind: MediaKind,
    id: &str,
    continuation: Option<&str>,
) -> Result<LibraryMedia, WebApiError> {
    PersonalService::media(ctx, kind, continuation)
        .await?
        .items
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| WebApiError::parse("资源已移除，请刷新相册"))
}

async fn resolve_current(
    resolver: &DataSessionResolver,
    recorder: &RuntimeTraceRecorderRef,
    kind: MediaKind,
    id: &str,
    continuation: Option<&str>,
) -> Result<LibraryMedia, DataError> {
    resolver
        .run_query(|session| async move {
            let ctx = PersonalService::context(&session, None, recorder.clone()).await?;
            resolve(&ctx, kind, id, continuation).await
        })
        .await
        .map_err(DataError::from)
}

pub async fn save(
    resolver: &DataSessionResolver,
    recorder: &RuntimeTraceRecorderRef,
    kind: MediaKind,
    id: &str,
    continuation: Option<&str>,
) -> Result<DataMediaSaveResult, DataError> {
    let media = resolve_current(resolver, recorder, kind, id, continuation).await?;
    let parsed = media_url(&media)?;
    let extension = if kind == MediaKind::Clip {
        "mp4"
    } else if parsed
        .path()
        .rsplit('.')
        .next()
        .is_some_and(|value| value.eq_ignore_ascii_case("png"))
    {
        "png"
    } else {
        "jpg"
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
    // 保存窗口可能停留很久；选定路径后重新恢复凭据并获取签名地址。
    let media = resolve_current(resolver, recorder, kind, id, continuation).await?;
    let parsed = media_url(&media)?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(300))
        .https_only(true)
        .build()
        .map_err(|_| "下载初始化失败")?;
    download_media(&client, parsed, &path, kind)
        .await
        .map_err(str::to_string)?;
    Ok(DataMediaSaveResult {
        saved: true,
        canceled: false,
    })
}

fn media_url(media: &LibraryMedia) -> Result<reqwest::Url, String> {
    let url = media.url.as_deref().ok_or("资源地址暂不可用，请刷新相册")?;
    let parsed = reqwest::Url::parse(url).map_err(|_| "资源地址无效")?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("资源地址无效".into());
    }
    Ok(parsed)
}

struct TemporaryDownload(PathBuf);
impl Drop for TemporaryDownload {
    fn drop(&mut self) {
        // future 取消同样清理临时文件，目标文件只在校验成功后原子替换。
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn download_media(
    client: &reqwest::Client,
    url: reqwest::Url,
    path: &Path,
    kind: MediaKind,
) -> Result<(), &'static str> {
    let temporary =
        TemporaryDownload(path.with_file_name(format!(".xbxrc-{}.download", uuid::Uuid::new_v4())));
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(media_request_error)?
        .error_for_status()
        .map_err(media_request_error)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MEDIA_BYTES)
    {
        return Err("媒体超过 2 GB 保存上限");
    }
    if let Some(content_type) = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    {
        let content_type = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let matches_kind = match kind {
            MediaKind::Clip => content_type.starts_with("video/"),
            MediaKind::Screenshot => content_type.starts_with("image/"),
        };
        if !matches_kind && content_type != "application/octet-stream" {
            return Err("资源响应格式异常，请刷新后重试");
        }
    }
    let mut file = tokio::fs::File::create(&temporary.0)
        .await
        .map_err(|_| "保存路径无法写入")?;
    let mut total = 0u64;
    let mut prefix = Vec::with_capacity(32);
    while let Some(chunk) = response.chunk().await.map_err(media_request_error)? {
        total = total.saturating_add(chunk.len() as u64);
        if total > MAX_MEDIA_BYTES {
            return Err("媒体超过 2 GB 保存上限");
        }
        prefix.extend_from_slice(&chunk[..chunk.len().min(32 - prefix.len())]);
        file.write_all(&chunk).await.map_err(|_| "媒体写入失败")?;
    }
    if total == 0 || !matches_media_signature(kind, &prefix) {
        return Err("资源响应格式异常，请刷新后重试");
    }
    file.flush().await.map_err(|_| "媒体写入失败")?;
    drop(file);
    tokio::fs::rename(&temporary.0, path)
        .await
        .map_err(|_| "媒体保存失败")?;
    Ok(())
}

fn media_request_error(error: reqwest::Error) -> &'static str {
    let error = WebApiError::from_reqwest_safe(error);
    log::warn!("[Data][mediaDownload] failed: {}", error.safe_summary());
    match error.to_status_code() {
        Some(401 | 403) => "资源已失效，请刷新后重试",
        Some(404) => "资源已移除，请刷新相册",
        Some(429) => "资源请求频繁，请稍后重试",
        Some(500..=599) => "资源服务暂不可用，请稍后重试",
        Some(_) => "资源下载失败，请刷新后重试",
        None => match error.network_error_kind() {
            Some(xbox_webapi::NetworkErrorKind::Dns) => "资源域名解析失败，请检查网络或 DNS",
            Some(xbox_webapi::NetworkErrorKind::Tls) => "资源安全连接失败，请检查系统时间和网络",
            Some(xbox_webapi::NetworkErrorKind::Timeout) => "资源下载超时，请稍后重试",
            Some(xbox_webapi::NetworkErrorKind::Body) => "资源下载已中断，请重试",
            _ => "资源下载失败，请检查网络后重试",
        },
    }
}

fn matches_media_signature(kind: MediaKind, prefix: &[u8]) -> bool {
    let ftyp = prefix.get(4..8) == Some(b"ftyp");
    let image_brand = matches!(
        prefix.get(8..12),
        Some(b"avif" | b"avis" | b"heic" | b"heix" | b"mif1")
    );
    match kind {
        MediaKind::Clip => ftyp && !image_brand,
        MediaKind::Screenshot => {
            prefix.starts_with(b"\x89PNG\r\n\x1a\n")
                || prefix.starts_with(b"\xff\xd8\xff")
                || prefix.starts_with(b"GIF87a")
                || prefix.starts_with(b"GIF89a")
                || (prefix.starts_with(b"RIFF") && prefix.get(8..12) == Some(b"WEBP"))
                || (ftyp && image_brand)
        }
    }
}

#[cfg(test)]
#[path = "media_save.test.rs"]
mod tests;
