use crate::{invalid, TouchBundleError, MAX_BUNDLE_BYTES, MAX_EXPANDED_BYTES, MAX_FILES};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};

pub(crate) fn read_archive(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, TouchBundleError> {
    if data.is_empty() || data.len() > MAX_BUNDLE_BYTES {
        return Err(invalid("官方布局包大小超出支持范围"));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|_| invalid("官方布局包需要为有效 TAKX ZIP 文件"))?;
    if zip.len() > MAX_FILES {
        return Err(invalid("布局包文件数量超出限制"));
    }
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for index in 0..zip.len() {
        let mut file = zip
            .by_index(index)
            .map_err(|_| invalid("布局包文件读取失败"))?;
        let path = archive_path(file.name())?;
        if file.is_symlink() {
            return Err(invalid("布局包包含符号链接"));
        }
        if file.is_dir() {
            continue;
        }
        let size = usize::try_from(file.size()).map_err(|_| invalid("布局包文件过大"))?;
        total = total
            .checked_add(size)
            .filter(|value| *value <= MAX_EXPANDED_BYTES)
            .ok_or_else(|| invalid("布局包展开大小超出限制"))?;
        let mut bytes = Vec::with_capacity(size);
        file.by_ref()
            .take(size as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("布局包文件完整性校验失败"))?;
        if bytes.len() != size || files.insert(path, bytes).is_some() {
            return Err(invalid("布局包文件身份或长度冲突"));
        }
    }
    Ok(files)
}

pub(crate) fn archive_path(path: &str) -> Result<String, TouchBundleError> {
    if path.is_empty() || path.len() > 512 || path.contains(['\\', ':', '\0']) {
        return Err(invalid("布局包文件路径无效"));
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.starts_with('/')
        || trimmed
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid("布局包路径超出资源目录"));
    }
    Ok(trimmed.to_ascii_lowercase())
}

pub(crate) fn relative_path(base: &str, target: &str) -> Result<String, TouchBundleError> {
    if target.starts_with('/') || target.contains(['\\', ':', '\0']) || target.len() > 512 {
        return Err(invalid("布局引用路径无效"));
    }
    let mut parts: Vec<&str> = base
        .rsplit_once('/')
        .map_or(Vec::new(), |(dir, _)| dir.split('/').collect());
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or_else(|| invalid("布局引用超出包目录"))?;
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/").to_ascii_lowercase())
}
