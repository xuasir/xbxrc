use crate::{HttpTransport, WebApiError};
use serde_json::Value;

/// better-xcloud 公共资源使用独立请求，版本固定到 gh-pages commit。
pub struct CommunityTouchApi {
    transport: HttpTransport,
}

impl Default for CommunityTouchApi {
    fn default() -> Self {
        Self {
            transport: HttpTransport::for_queries(),
        }
    }
}

impl CommunityTouchApi {
    pub async fn latest_revision(&self) -> Result<String, WebApiError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("XBXRC"),
        );
        let bytes = self
            .transport
            .get_bytes(
                "https://api.github.com/repos/redphx/better-xcloud/branches/gh-pages",
                Some(headers),
                256 * 1024,
            )
            .await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| WebApiError::parse("invalid community revision response"))?;
        let revision = value["commit"]["sha"]
            .as_str()
            .filter(|value| valid_revision(value))
            .ok_or_else(|| WebApiError::parse("invalid community revision"))?;
        Ok(revision.to_owned())
    }

    pub async fn definition(
        &self,
        revision: &str,
        title_id: &str,
    ) -> Result<Option<Value>, WebApiError> {
        if !title_id.parse::<u64>().is_ok_and(|value| value > 0) {
            return Err(WebApiError::parse("invalid community title identity"));
        }
        match self
            .json(revision, &format!("touch-layouts/{title_id}.json"))
            .await
        {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.to_status_code() == Some(404) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn layouts(
        &self,
        revision: &str,
        definition: &Value,
    ) -> Result<Vec<Value>, WebApiError> {
        let names = definition["layouts"]
            .as_array()
            .filter(|names| !names.is_empty() && names.len() <= 16)
            .ok_or_else(|| WebApiError::parse("invalid community layout references"))?;
        let mut layouts = Vec::new();
        for name in names {
            let name = name
                .as_str()
                .filter(|name| valid_name(name))
                .ok_or_else(|| WebApiError::parse("invalid community layout name"))?;
            // 逐份等待合并，完成后才交付目录与保存包。
            layouts.push(
                self.json(revision, &format!("touch-layouts/layouts/{name}.json"))
                    .await?,
            );
        }
        Ok(layouts)
    }

    async fn json(&self, revision: &str, path: &str) -> Result<Value, WebApiError> {
        if !valid_revision(revision) {
            return Err(WebApiError::parse("invalid community revision"));
        }
        let url =
            format!("https://raw.githubusercontent.com/redphx/better-xcloud/{revision}/{path}");
        let bytes = self
            .transport
            .get_bytes(&url, None, 2 * 1024 * 1024)
            .await?;
        serde_json::from_slice(&bytes)
            .map_err(|_| WebApiError::parse("invalid community layout JSON"))
    }
}

fn valid_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_source_paths_to_pinned_repository_files() {
        assert!(valid_revision("4416b1ab07588a3244a5c02ce1fa5dfa64d8df74"));
        assert!(valid_name("forza"));
        for name in ["../forza", "https://example.org", "forza.json", "", "a/b"] {
            assert!(!valid_name(name));
        }
        assert!(!valid_revision("refs/heads/gh-pages"));
    }
}
