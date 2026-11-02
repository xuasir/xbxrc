use serde_json::Value;

/// 更新源和签名之外继续核验发行身份，避免不同应用安装身份互相覆盖。
pub fn validate_update_manifest(
    manifest: &Value,
    channel: &str,
    edition: &str,
    identifier: &str,
) -> Result<(), String> {
    if manifest["edition"].as_str() != Some(edition)
        || manifest["identifier"].as_str() != Some(identifier)
    {
        return Err("更新清单的应用身份与当前发行版本不符".to_owned());
    }
    let commit = manifest["sourceCommit"].as_str().unwrap_or_default();
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("更新清单缺少源码提交身份".to_owned());
    }
    let version = manifest["version"].as_str().unwrap_or_default();
    let (base, suffix) = version.split_once('-').unwrap_or((version, ""));
    let parts: Vec<_> = base.split('.').collect();
    let base_valid = parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        });
    let preview_valid = suffix
        .strip_prefix(&format!("{channel}."))
        .is_some_and(|run| {
            !run.is_empty()
                && !run.starts_with('0')
                && run.bytes().all(|byte| byte.is_ascii_digit())
        });
    let channel_valid = match (edition, channel) {
        ("browser", "stable") => suffix.is_empty(),
        ("browser", "beta") | ("native-wip", "native-wip") => preview_valid,
        _ => false,
    };
    if !base_valid || !channel_valid {
        return Err("更新清单的版本与当前渠道不符".to_owned());
    }
    let tag = if channel == "stable" {
        format!("v{version}")
    } else {
        channel.to_owned()
    };
    let prefix = format!("https://github.com/xuasir/xbxrc/releases/download/{tag}/");
    let platforms = manifest["platforms"]
        .as_object()
        .filter(|platforms| !platforms.is_empty())
        .ok_or_else(|| "更新清单缺少平台资产".to_owned())?;
    for platform in platforms.values() {
        let url = platform["url"].as_str().unwrap_or_default();
        let filename = url.strip_prefix(&prefix).unwrap_or_default();
        if filename.is_empty()
            || filename.contains(['/', '?', '#', '%'])
            || !filename.contains(version)
        {
            return Err("更新资产地址与当前渠道不符".to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(edition: &str, channel: &str) -> Value {
        let version = if channel == "stable" {
            "0.1.0".to_owned()
        } else {
            format!("0.1.0-{channel}.8")
        };
        let tag = if channel == "stable" {
            format!("v{version}")
        } else {
            channel.to_owned()
        };
        json!({
            "edition": edition, "identifier": "app-id", "sourceCommit": "a".repeat(40),
            "version": version, "platforms": {
                "darwin-aarch64": { "url": format!("https://github.com/xuasir/xbxrc/releases/download/{tag}/app-{version}.tar.gz") }
            }
        })
    }

    #[test]
    fn accepts_each_editions_channel() {
        for (edition, channel) in [
            ("browser", "stable"),
            ("browser", "beta"),
            ("native-wip", "native-wip"),
        ] {
            assert!(validate_update_manifest(
                &manifest(edition, channel),
                channel,
                edition,
                "app-id"
            )
            .is_ok());
        }
    }

    #[test]
    fn rejects_identity_and_version_mismatches() {
        let valid = manifest("browser", "stable");
        for (field, value) in [
            ("edition", "native-wip"),
            ("identifier", "other"),
            ("sourceCommit", "unknown"),
            ("version", "0.1.0-beta.8"),
        ] {
            let mut changed = valid.clone();
            changed[field] = json!(value);
            assert!(validate_update_manifest(&changed, "stable", "browser", "app-id").is_err());
        }
        assert!(validate_update_manifest(&valid, "stable", "native-wip", "app-id").is_err());
    }

    #[test]
    fn rejects_cross_channel_asset_urls() {
        let mut changed = manifest("native-wip", "native-wip");
        changed["platforms"]["darwin-aarch64"]["url"] =
            json!("https://github.com/xuasir/xbxrc/releases/download/beta/app.tar.gz");
        assert!(validate_update_manifest(&changed, "native-wip", "native-wip", "app-id").is_err());
    }
}
