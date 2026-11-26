use super::{
    invalid, types::GameIdentity, XboxSavedTouchLayout, XboxTouchLayoutAsset,
    XboxTouchLayoutSnapshot,
};
use crate::XboxBridgeError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use xbox_touch_layouts::{
    compile_layout, content_hash, protocol_revision, CompiledTouchLayout, ResolvedTouchLayout,
    TouchBundle, MAX_BUNDLE_BYTES, MAX_COMPILED_BYTES, MAX_EXPANDED_BYTES, MAX_JSON_BYTES,
};

const MAX_SAVED_LAYOUTS: usize = 100;
const MAX_LIBRARY_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolCache {
    source_revision: String,
    protocol_revision: String,
    checksum: String,
    layout: CompiledTouchLayout,
}

fn cache_filename(revision: &str) -> String {
    format!("{revision}.{}.ir.json", protocol_revision())
}

fn cache_bytes(layout: CompiledTouchLayout, revision: &str) -> Result<Vec<u8>, XboxBridgeError> {
    let checksum =
        content_hash(&serde_json::to_vec(&layout).map_err(|_| invalid("协议快照编码失败"))?);
    let bytes = serde_json::to_vec(&ProtocolCache {
        source_revision: revision.into(),
        protocol_revision: protocol_revision().into(),
        checksum,
        layout,
    })
    .map_err(|_| invalid("协议缓存编码失败"))?;
    if bytes.len() > MAX_COMPILED_BYTES + 64 * 1024 {
        return Err(invalid("协议缓存超过8MB预算"));
    }
    Ok(bytes)
}

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
    let mut entries = read_index(root)?;
    let previous_entries = entries.clone();
    let hash = content_hash(original);
    let compiled = serde_json::to_vec(layout).map_err(|_| invalid("布局快照序列化失败"))?;
    if compiled.len() > MAX_JSON_BYTES + 64 * 1024 {
        return Err(invalid("布局快照超过保存大小限制"));
    }
    let revision = content_hash(&compiled);
    let ir = compile_layout(
        &serde_json::to_string(&layout.document).map_err(|_| invalid("布局来源编码失败"))?,
        &layout.id,
        if bundle_id.starts_with("community:") {
            "community"
        } else {
            "official"
        },
        &layout
            .assets
            .iter()
            .map(|asset| asset.id.clone())
            .collect::<Vec<_>>(),
        None,
        None,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    if !ir.is_runnable {
        return Err(invalid("此布局需要有效的输入状态后保存"));
    }
    let protocol_cache = cache_bytes(ir, &revision)?;
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
                (compiled.len() + protocol_cache.len()) as u64
            } else {
                fs::metadata(
                    root.join("packages")
                        .join(&entry.content_hash)
                        .join("compiled")
                        .join(format!("{}.json", entry.revision)),
                )
                .map(|metadata| metadata.len())
                .unwrap_or(0)
                    + fs::metadata(
                        root.join("packages")
                            .join(&entry.content_hash)
                            .join("compiled")
                            .join(cache_filename(&entry.revision)),
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
    // 来源修订继续用于本机身份；协议缓存独立换代，保持已选布局稳定。
    let _ = write_atomic(
        &package.join("compiled").join(cache_filename(&revision)),
        &protocol_cache,
    );
    collect_unused(root, &entries)?;
    read_index(root)
}

pub(super) fn load(root: &Path, id: &str) -> Result<XboxTouchLayoutSnapshot, XboxBridgeError> {
    let entries = read_index(root)?;
    let entry = entries
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
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
    let mut issues = layout.issues.clone();
    let mut assets = Vec::new();
    for asset in &layout.assets {
        let path = resource_path(&asset.path)?;
        let Ok(data) = read(&package.join("expanded").join(path), MAX_EXPANDED_BYTES) else {
            issues.push("此本机图片缺失，使用默认图形".into());
            continue;
        };
        if content_hash(&data) != asset.content_hash {
            issues.push("此本机图片校验失败，使用默认图形".into());
            continue;
        }
        total = total
            .checked_add(data.len())
            .filter(|size| *size <= MAX_EXPANDED_BYTES)
            .ok_or_else(|| invalid("布局图片总量超出限制"))?;
        assets.push(XboxTouchLayoutAsset {
            id: asset.id.clone(),
            data,
        });
    }
    let mut asset_ids: Vec<_> = assets.iter().map(|asset| asset.id.clone()).collect();
    asset_ids.sort();
    let cache_path = package
        .join("compiled")
        .join(cache_filename(&entry.revision));
    let cached = read(&cache_path, MAX_COMPILED_BYTES + 64 * 1024)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ProtocolCache>(&bytes).ok())
        .filter(|cache| {
            cache.source_revision == entry.revision
                && cache.protocol_revision == protocol_revision()
                && cache.layout.protocol_revision == protocol_revision()
                && cache.layout.id == layout.id
                && cache.layout.available_assets == asset_ids
                && serde_json::to_vec(&cache.layout)
                    .is_ok_and(|bytes| content_hash(&bytes) == cache.checksum)
        });
    let ir = if let Some(cache) = cached {
        cache.layout
    } else {
        let ir = compile_layout(
            &serde_json::to_string(&layout.document).map_err(|_| invalid("布局来源编码失败"))?,
            &layout.id,
            if entry.bundle_id.starts_with("community:") {
                "community"
            } else {
                "official"
            },
            &asset_ids,
            None,
            None,
        )
        .map_err(|error| invalid(&error.to_string()))?;
        let bytes = cache_bytes(ir.clone(), &entry.revision)?;
        if cache_fits(root, &entries, &entry, bytes.len()) {
            if let Err(error) = write_atomic(&cache_path, &bytes) {
                issues.push(format!("协议缓存使用本次编译结果：{error}"));
            }
        } else {
            issues.push("本机缓存达到预算，使用本次编译结果".into());
        }
        ir
    };
    Ok(XboxTouchLayoutSnapshot {
        id: entry.id,
        name: entry.layout_name,
        document_json: serde_json::to_string(&ir).map_err(|_| invalid("布局快照序列化失败"))?,
        assets,
        issues,
    })
}

pub(super) fn delete(root: &Path, id: &str) -> Result<Vec<XboxSavedTouchLayout>, XboxBridgeError> {
    let mut entries = read_index(root)?;
    entries.retain(|entry| entry.id != id);
    write_index(root, &entries)?;
    collect_unused(root, &entries)?;
    Ok(entries)
}

fn cache_fits(
    root: &Path,
    entries: &[XboxSavedTouchLayout],
    current: &XboxSavedTouchLayout,
    new_bytes: usize,
) -> bool {
    let mut packages = BTreeSet::new();
    let mut revisions = BTreeSet::new();
    let mut total = 0u64;
    for entry in entries {
        if packages.insert(&entry.content_hash) {
            total = total.saturating_add(entry.byte_count);
        }
        if revisions.insert((&entry.content_hash, &entry.revision)) {
            let folder = root
                .join("packages")
                .join(&entry.content_hash)
                .join("compiled");
            total = total.saturating_add(
                fs::metadata(folder.join(format!("{}.json", entry.revision)))
                    .map(|m| m.len())
                    .unwrap_or(0),
            );
            total = total.saturating_add(
                if entry.content_hash == current.content_hash && entry.revision == current.revision
                {
                    new_bytes as u64
                } else {
                    fs::metadata(folder.join(cache_filename(&entry.revision)))
                        .map(|m| m.len())
                        .unwrap_or(0)
                },
            );
        }
    }
    total <= MAX_LIBRARY_BYTES
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
            .flat_map(|entry| {
                [
                    format!("{}.json", entry.revision),
                    cache_filename(&entry.revision),
                ]
            })
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
