use crate::error::WebApiError;
use crate::transport::HttpTransport;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

pub struct XcloudApi {
    transport: HttpTransport,
    host: String,
    bearer_token: String,
}

impl XcloudApi {
    pub fn new(host: String, bearer_token: String) -> Self {
        Self {
            transport: HttpTransport::for_queries(),
            host,
            bearer_token,
        }
    }

    pub fn with_transport(transport: HttpTransport, host: String, bearer_token: String) -> Self {
        Self {
            transport,
            host,
            bearer_token,
        }
    }

    fn authorization_header(&self) -> String {
        format!("Bearer {}", self.bearer_token)
    }

    fn headers(&self) -> Result<HeaderMap, WebApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header())
                .map_err(|_| WebApiError::parse("invalid cloud authorization"))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }

    pub async fn get_titles_by_ids(
        &self,
        ids: &[String],
        id_type: &str,
    ) -> Result<Value, WebApiError> {
        let payload = title_ids_payload(ids, id_type)?;
        self.transport
            .post_query(&titles_url(&self.host)?, payload, Some(self.headers()?))
            .await
    }

    pub async fn input_configs(&self, xbox_title_id: &str) -> Result<Value, WebApiError> {
        if xbox_title_id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .is_none()
        {
            return Err(WebApiError::parse("invalid Xbox title identity"));
        }
        let mut url = normalized_origin(&self.host)?;
        url.set_path("/v2/titles/inputconfigs");
        self.transport
            .post_query(
                url.as_str(),
                serde_json::json!({
                    "titleIds": [xbox_title_id], "titleIdType": "xboxTitleId"
                }),
                Some(self.headers()?),
            )
            .await
    }

    /// 官方下载先返回带有效期的资源地址，资源请求使用地址自身的授权。
    pub async fn touch_adaptation_bundle(&self, bundle_id: &str) -> Result<Vec<u8>, WebApiError> {
        let url = touch_bundle_url(&self.host, bundle_id)?;
        let address = self
            .transport
            .get_bytes(&url, Some(self.headers()?), 8192)
            .await?;
        let address = std::str::from_utf8(&address)
            .map_err(|_| WebApiError::parse("invalid touch bundle download address"))?
            .trim();
        let address = validate_touch_download_address(address)?;
        self.transport
            .get_bytes(address.as_str(), None, 16 * 1024 * 1024)
            .await
    }

    pub async fn get_waiting_time(&self, stream_title_id: &str) -> Result<Value, WebApiError> {
        if stream_title_id.trim().is_empty() || stream_title_id.len() > 256 {
            return Err(WebApiError::parse("invalid stream title identity"));
        }
        let mut url = normalized_origin(&self.host)?;
        url.path_segments_mut()
            .map_err(|_| WebApiError::parse("invalid cloud origin"))?
            .extend(["v1", "waittime", stream_title_id]);
        self.transport
            .get(url.as_str(), Some(self.headers()?))
            .await
    }

    pub async fn get_titles(&self) -> Result<Value, WebApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header())
                .map_err(|e| WebApiError::parse(e.to_string()))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let url = titles_url(&self.host)?;
        self.transport.get(&url, Some(headers)).await
    }

    pub async fn get_recent_titles(&self, max_results: u32) -> Result<Value, WebApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header())
                .map_err(|e| WebApiError::parse(e.to_string()))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let url = recent_titles_url(&self.host, max_results)?;
        self.transport.get(&url, Some(headers)).await
    }
}

fn normalized_origin(host: &str) -> Result<reqwest::Url, WebApiError> {
    let host = host.trim();
    if host.is_empty() || host.contains('\\') || host.chars().any(char::is_whitespace) {
        return Err(WebApiError::parse("invalid xCloud region host"));
    }
    let origin = if host.contains("://") {
        host.to_string()
    } else {
        format!("https://{host}")
    };
    let mut url = reqwest::Url::parse(&origin)
        .map_err(|_| WebApiError::parse("invalid xCloud region host"))?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WebApiError::parse("invalid xCloud region host"));
    }
    url.set_scheme("https")
        .map_err(|_| WebApiError::parse("invalid xCloud region scheme"))?;
    Ok(url)
}

fn titles_url(host: &str) -> Result<String, WebApiError> {
    let mut url = normalized_origin(host)?;
    url.set_path("/v2/titles");
    Ok(url.into())
}

fn title_ids_payload(ids: &[String], id_type: &str) -> Result<Value, WebApiError> {
    if !matches!(id_type, "productId" | "titleId") || ids.is_empty() || ids.len() > 75 {
        return Err(WebApiError::parse("invalid cloud title query"));
    }
    // 云目录使用 alternate identity 合同，商品 ID 经此查询转换为 Xbox TitleId。
    Ok(serde_json::json!({"alternateIds": ids, "alternateIdType": id_type}))
}

fn touch_bundle_url(host: &str, bundle_id: &str) -> Result<String, WebApiError> {
    if bundle_id.is_empty()
        || bundle_id.len() > 128
        || !bundle_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_' | b'.'))
        || matches!(bundle_id, "." | "..")
    {
        return Err(WebApiError::parse("invalid touch bundle identity"));
    }
    let mut url = normalized_origin(host)?;
    url.path_segments_mut()
        .map_err(|_| WebApiError::parse("invalid cloud origin"))?
        .extend(["v1", "sessions", "tabs", bundle_id, "download"]);
    url.query_pairs_mut().append_pair("noRedirect", "true");
    Ok(url.into())
}

fn validate_touch_download_address(address: &str) -> Result<reqwest::Url, WebApiError> {
    let url = reqwest::Url::parse(address)
        .map_err(|_| WebApiError::parse("invalid touch bundle download address"))?;
    let valid_host = url.host_str().is_some_and(|host| {
        [
            "blob.core.windows.net",
            "azureedge.net",
            "azurefd.net",
            "xboxlive.com",
            "xbox.com",
            "microsoft.com",
        ]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    });
    if url.scheme() != "https"
        || !valid_host
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err(WebApiError::parse("invalid touch bundle download address"));
    }
    Ok(url)
}

fn recent_titles_url(host: &str, max_results: u32) -> Result<String, WebApiError> {
    let mut url = normalized_origin(host)?;
    url.set_path("/v2/titles/mru");
    url.query_pairs_mut()
        .append_pair("mr", &max_results.to_string());
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::{
        recent_titles_url, title_ids_payload, titles_url, touch_bundle_url,
        validate_touch_download_address,
    };

    #[test]
    fn product_identity_query_matches_official_sdk_contract() {
        assert_eq!(
            title_ids_payload(&["9MWRLK77D9F4".into()], "productId").unwrap(),
            serde_json::json!({
                "alternateIds": ["9MWRLK77D9F4"],
                "alternateIdType": "productId"
            })
        );
        assert!(title_ids_payload(&[], "productId").is_err());
        assert!(title_ids_payload(&["9MWRLK77D9F4".into()], "xboxTitleId").is_err());
    }

    #[test]
    fn official_touch_bundle_uses_authenticated_address_then_signed_resource() {
        assert_eq!(touch_bundle_url("wus.core.gssv-play-prod.xboxlive.com", "bundle-id").unwrap(),
            "https://wus.core.gssv-play-prod.xboxlive.com/v1/sessions/tabs/bundle-id/download?noRedirect=true");
        assert!(validate_touch_download_address(
            "https://touch.blob.core.windows.net/bundle.takx?sig=fixture"
        )
        .is_ok());
        for invalid in [
            "http://touch.blob.core.windows.net/bundle",
            "https://blob.core.windows.net.other.test/bundle",
            "https://user:secret@touch.blob.core.windows.net/bundle",
            "https://127.0.0.1/bundle",
        ] {
            assert!(validate_touch_download_address(invalid).is_err());
        }
        for invalid in ["", "..", "../bundle", "bundle?key=one"] {
            assert!(touch_bundle_url("example.com", invalid).is_err());
        }
    }

    #[test]
    fn builds_xcloud_catalog_urls() {
        assert_eq!(
            titles_url("https://wus.core.gssv-play-prod.xboxlive.com/").expect("titles url"),
            "https://wus.core.gssv-play-prod.xboxlive.com/v2/titles"
        );
        assert_eq!(
            recent_titles_url("wus.core.gssv-play-prod.xboxlive.com", 25)
                .expect("recent titles url"),
            "https://wus.core.gssv-play-prod.xboxlive.com/v2/titles/mru?mr=25"
        );
    }

    #[test]
    fn rejects_invalid_region_hosts() {
        for host in [
            "",
            "example.com/path",
            "example.com?token=x",
            "example.com#fragment",
            "https://user:secret@example.com",
            "user@example.com",
            "example.com\\path",
            "https://example.com\\@other.example",
            "ftp://example.com",
            "example .com",
        ] {
            assert!(titles_url(host).is_err(), "unexpected valid host: {host}");
        }
    }

    #[test]
    fn normalizes_supported_origin_and_builds_only_https_requests() {
        assert_eq!(
            titles_url(" http://example.com/ ").unwrap(),
            "https://example.com/v2/titles"
        );
        assert_eq!(
            titles_url("https://example.com:8443").unwrap(),
            "https://example.com:8443/v2/titles"
        );
    }
}
