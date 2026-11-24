use super::{
    invalid, types::GameIdentity, XboxSavedTouchLayout, XboxTouchLayoutAsset,
    XboxTouchLayoutSnapshot,
};
use crate::XboxBridgeError;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use xbox_touch_layouts::{
    content_hash, ResolvedTouchLayout, TouchBundle, MAX_BUNDLE_BYTES, MAX_EXPANDED_BYTES,
    MAX_JSON_BYTES,
};

const MAX_SAVED_LAYOUTS: usize = 100;
const MAX_LIBRARY_BYTES: u64 = 512 * 1024 * 1024;

pub(super) fn read_index(root: &Path) -> Result<Vec<XboxSavedTouchLayout>, XboxBridgeError> {
    let path = root.join("index.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_slice(&read(&path, MAX_JSON_BYTES)?)
        .map_err(|_| invalid("本机布局索引损坏"))?;
    if value["version"] != 1 {
        return Err(invalid("本机布局索引版本等待支持"));
    }
    let source = value["layouts"]
        .as_array()
        .ok_or_else(|| invalid("本机布局索引无效"))?;
    let mut ids = BTreeSet::new();
    let mut result: Vec<XboxSavedTouchLayout> = source
        .iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .filter(|entry: &XboxSavedTouchLayout| valid_entry(entry) && ids.insert(entry.id.clone()))
        .take(MAX_SAVED_LAYOUTS)
        .collect();
    result.sort_by(|left, right| {
        right
            .last_used_at_ms
            .cmp(&left.last_used_at_ms)
            .then_with(|| left.game_name.cmp(&right.game_name))
            .then_with(|| left.layout_name.cmp(&right.layout_name))
    });
    Ok(result)
}

pub(super) fn save(
    root: &Path,
    identity: &GameIdentity,
    bundle_id: &str,
    original: &[u8],
    bundle: &TouchBundle,
    layout_id: &str,
) -> Result<Vec<XboxSavedTouchLayout>, XboxBridgeError> {
    let layout = bundle
        .layouts
        .iter()
        .find(|layout| layout.id == layout_id)
        .ok_or_else(|| invalid("布局身份无效"))?;
    if !layout.issues.is_empty() {
        return Err(invalid("此布局需要完成资源适配后保存"));
    }
    let mut entries = read_index(root)?;
    let previous_entries = entries.clone();
    let hash = content_hash(original);
    let compiled = serde_json::to_vec(layout).map_err(|_| invalid("布局快照序列化失败"))?;
    if compiled.len() > MAX_JSON_BYTES + 64 * 1024 {
        return Err(invalid("布局快照超过保存大小限制"));
    }
    let revision = content_hash(&compiled);
    // 社区 commit 用于内容版本；同来源的更新沿原保存身份替换。
    let community = bundle_id.starts_with("community:better-xcloud:");
    let bundle_id = if community {
        "community:better-xcloud"
    } else {
        bundle_id
    };
    let id = format!(
        "{}:{}",
        if community { "community" } else { "official" },
        content_hash(format!("{}\0{bundle_id}\0{layout_id}", identity.title_id).as_bytes())
    );
    let previous = entries.iter().find(|entry| entry.id == id);
    let byte_count = (original.len() + bundle.files.values().map(Vec::len).sum::<usize>()) as u64;
    let entry = XboxSavedTouchLayout {
        id: id.clone(),
        product_id: identity.product_id.clone(),
        xbox_title_id: identity.title_id.clone(),
        game_name: identity.name.clone(),
        bundle_id: bundle_id.into(),
        bundle_version: bundle.version.clone(),
        layout_id: layout.id.clone(),
        layout_name: layout.name.clone(),
        is_default: layout.is_default,
        content_hash: hash.clone(),
        revision: revision.clone(),
        saved_at_ms: previous.map_or_else(now, |entry| entry.saved_at_ms),
        last_used_at_ms: previous.map_or(0, |entry| entry.last_used_at_ms),
        byte_count,
    };
    entries.retain(|entry| entry.id != id);
    entries.push(entry);
    let mut hashes = BTreeSet::new();
    let package_bytes: u64 = entries
        .iter()
        .filter(|entry| hashes.insert(entry.content_hash.clone()))
        .map(|entry| entry.byte_count)
        .sum();
    let mut revisions = BTreeSet::new();
    let compiled_bytes: u64 = entries
        .iter()
        .filter(|entry| revisions.insert((entry.content_hash.clone(), entry.revision.clone())))
        .map(|entry| {
            if entry.content_hash == hash && entry.revision == revision {
                compiled.len() as u64
            } else {
                fs::metadata(
                    root.join("packages")
                        .join(&entry.content_hash)
                        .join("compiled")
                        .join(format!("{}.json", entry.revision)),
                )
                .map(|metadata| metadata.len())
                .unwrap_or(0)
            }
        })
        .sum();
    let total = package_bytes.saturating_add(compiled_bytes);
    if entries.len() > MAX_SAVED_LAYOUTS || total > MAX_LIBRARY_BYTES {
        return Err(invalid("本机布局库已达保存上限"));
    }
    fs::create_dir_all(root.join("packages")).map_err(|_| invalid("布局目录创建失败"))?;
    let package = root.join("packages").join(&hash);
    if !package.exists() {
        let pending = PendingDirectory::new(root)?;
        write(&pending.0.join("original.takx"), original)?;
        for (path, data) in &bundle.files {
            write(&pending.0.join("expanded").join(path), data)?;
        }
        fs::rename(&pending.0, &package).map_err(|_| invalid("布局资源提交失败"))?;
    } else {
        // 同版本重新保存可修复缺失或损坏的原件和资源，已有快照继续保留。
        repair(&package.join("original.takx"), original, MAX_BUNDLE_BYTES)?;
        for (path, data) in &bundle.files {
            repair(
                &package.join("expanded").join(path),
                data,
                MAX_EXPANDED_BYTES,
            )?;
        }
    }
    if let Err(error) = write_atomic(
        &package.join("compiled").join(format!("{revision}.json")),
        &compiled,
    ) {
        let _ = collect_unused(root, &previous_entries);
        return Err(error);
    }
    if let Err(error) = write_index(root, &entries) {
        let _ = collect_unused(root, &previous_entries);
        return Err(error);
    }
    collect_unused(root, &entries)?;
    read_index(root)
}

pub(super) fn load(root: &Path, id: &str) -> Result<XboxTouchLayoutSnapshot, XboxBridgeError> {
    let entry = read_index(root)?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| invalid("本机布局已移除"))?;
    let package = root.join("packages").join(&entry.content_hash);
    let bytes = read(
        &package
            .join("compiled")
            .join(format!("{}.json", entry.revision)),
        MAX_JSON_BYTES + 64 * 1024,
    )?;
    if content_hash(&bytes) != entry.revision {
        return Err(invalid("本机布局完整性校验失败"));
    }
    let layout: ResolvedTouchLayout =
        serde_json::from_slice(&bytes).map_err(|_| invalid("本机布局快照损坏"))?;
    if layout.id != entry.layout_id {
        return Err(invalid("本机布局身份校验失败"));
    }
    let mut total = 0usize;
    let assets = layout
        .assets
        .into_iter()
        .map(|asset| {
            let path = resource_path(&asset.path)?;
            let data = read(&package.join("expanded").join(path), MAX_EXPANDED_BYTES)?;
            if content_hash(&data) != asset.content_hash {
                return Err(invalid("本机布局图片完整性校验失败"));
            }
            total = total
                .checked_add(data.len())
                .filter(|size| *size <= MAX_EXPANDED_BYTES)
                .ok_or_else(|| invalid("布局图片总量超出限制"))?;
            Ok(XboxTouchLayoutAsset { id: asset.id, data })
        })
        .collect::<Result<Vec<_>, XboxBridgeError>>()?;
    Ok(XboxTouchLayoutSnapshot {
        id: entry.id,
        name: entry.layout_name,
        document_json: serde_json::to_string(&layout.document)
            .map_err(|_| invalid("布局快照序列化失败"))?,
        assets,
        issues: layout.issues,
    })
}

pub(super) fn delete(root: &Path, id: &str) -> Result<Vec<XboxSavedTouchLayout>, XboxBridgeError> {
    let mut entries = read_index(root)?;
    entries.retain(|entry| entry.id != id);
    write_index(root, &entries)?;
    collect_unused(root, &entries)?;
    Ok(entries)
}

pub(super) fn mark_used(root: &Path, id: &str) -> Result<(), XboxBridgeError> {
    let mut entries = read_index(root)?;
    if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) {
        entry.last_used_at_ms = now();
        write_index(root, &entries)?;
    }
    Ok(())
}

fn valid_entry(entry: &XboxSavedTouchLayout) -> bool {
    entry
        .id
        .strip_prefix("official:")
        .or_else(|| entry.id.strip_prefix("community:"))
        .is_some_and(valid_hash)
        && valid_hash(&entry.content_hash)
        && valid_hash(&entry.revision)
        && !entry.layout_id.is_empty()
        && entry.layout_id.len() <= 128
        && entry.game_name.chars().count() <= 120
        && entry
            .xbox_title_id
            .parse::<u64>()
            .is_ok_and(|value| value > 0)
        && entry.byte_count <= (MAX_EXPANDED_BYTES + MAX_BUNDLE_BYTES) as u64
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn resource_path(value: &str) -> Result<PathBuf, XboxBridgeError> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(['\\', ':', '\0'])
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid("本机布局资源路径无效"));
    }
    Ok(PathBuf::from(value))
}

fn read(path: &Path, limit: usize) -> Result<Vec<u8>, XboxBridgeError> {
    let file = fs::File::open(path).map_err(|_| invalid("本机布局文件读取失败"))?;
    let size = file
        .metadata()
        .map_err(|_| invalid("布局文件信息读取失败"))?
        .len();
    if size > limit as u64 {
        return Err(invalid("本机布局文件超出大小限制"));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("本机布局文件读取失败"))?;
    if bytes.len() > limit {
        return Err(invalid("本机布局文件超出大小限制"));
    }
    Ok(bytes)
}

fn write(path: &Path, data: &[u8]) -> Result<(), XboxBridgeError> {
    fs::create_dir_all(path.parent().ok_or_else(|| invalid("布局写入路径无效"))?)
        .map_err(|_| invalid("布局目录创建失败"))?;
    fs::write(path, data).map_err(|_| invalid("布局文件写入失败"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), XboxBridgeError> {
    let temporary = path.with_extension(format!("pending-{}", uuid::Uuid::new_v4()));
    if let Err(error) = write(&temporary, bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if fs::rename(&temporary, path).is_err() {
        let _ = fs::remove_file(temporary);
        return Err(invalid("布局文件提交失败"));
    }
    Ok(())
}

fn repair(path: &Path, expected: &[u8], limit: usize) -> Result<(), XboxBridgeError> {
    if read(path, limit).is_ok_and(|data| data == expected) {
        return Ok(());
    }
    write_atomic(path, expected)
}

fn write_index(root: &Path, entries: &[XboxSavedTouchLayout]) -> Result<(), XboxBridgeError> {
    let bytes = serde_json::to_vec(&json!({"version":1,"layouts":entries}))
        .map_err(|_| invalid("布局索引序列化失败"))?;
    write_atomic(&root.join("index.json"), &bytes)
}

pub(super) fn collect_unused(
    root: &Path,
    entries: &[XboxSavedTouchLayout],
) -> Result<(), XboxBridgeError> {
    // 索引提交后按引用回收；读取列表时重试上次中断的清理。
    let contents = match fs::read_dir(root) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(invalid("本机布局资源清理失败，请重试")),
    };
    for item in contents {
        let item = item.map_err(|_| invalid("本机布局资源清理失败，请重试"))?;
        let name = item.file_name().to_string_lossy().to_string();
        if name
            .strip_prefix(".pending-")
            .or_else(|| name.strip_prefix("index.pending-"))
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        {
            remove_resource(&item.path())?;
        }
    }
    let packages = match fs::read_dir(root.join("packages")) {
        Ok(packages) => packages,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(invalid("本机布局资源清理失败，请重试")),
    };
    for package in packages {
        let package = package.map_err(|_| invalid("本机布局资源清理失败，请重试"))?;
        let hash = package.file_name().to_string_lossy().to_string();
        if !valid_hash(&hash) {
            continue;
        }
        let revisions: BTreeSet<_> = entries
            .iter()
            .filter(|entry| entry.content_hash == hash)
            .map(|entry| format!("{}.json", entry.revision))
            .collect();
        if revisions.is_empty() {
            remove_resource(&package.path())?;
            continue;
        }
        let files = match fs::read_dir(package.path().join("compiled")) {
            Ok(files) => files,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(invalid("本机布局资源清理失败，请重试")),
        };
        for file in files {
            let file = file.map_err(|_| invalid("本机布局资源清理失败，请重试"))?;
            if !revisions.contains(file.file_name().to_string_lossy().as_ref()) {
                remove_resource(&file.path())?;
            }
        }
    }
    Ok(())
}

fn remove_resource(path: &Path) -> Result<(), XboxBridgeError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| invalid("本机布局资源清理失败，请重试"))?;
    let result = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|_| invalid("本机布局资源清理失败，请重试"))
}

struct PendingDirectory(PathBuf);
impl PendingDirectory {
    fn new(root: &Path) -> Result<Self, XboxBridgeError> {
        let path = root.join(format!(".pending-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).map_err(|_| invalid("布局准备目录创建失败"))?;
        Ok(Self(path))
    }
}
impl Drop for PendingDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
