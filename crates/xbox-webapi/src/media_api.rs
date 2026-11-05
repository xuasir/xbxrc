use crate::{
    parse_media_page, HttpTransport, LibraryMedia, LibraryPage, MediaKind, RetryMode, WebApiError,
};
use serde_json::json;

pub struct MediaApi {
    transport: HttpTransport,
    uhs: String,
    token: String,
}
impl MediaApi {
    pub fn new(uhs: String, token: String) -> Self {
        Self {
            transport: HttpTransport::with_retry_mode(RetryMode::Always),
            uhs,
            token,
        }
    }
    pub async fn search(
        &self,
        xuid: &str,
        kind: MediaKind,
        continuation: Option<&str>,
    ) -> Result<LibraryPage<LibraryMedia>, WebApiError> {
        let (url, body) = media_request(xuid, kind, continuation)?;
        let headers = HttpTransport::create_header_map(&[
            (
                "Authorization",
                &format!("XBL3.0 x={};{}", self.uhs, self.token),
            ),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-xbl-contract-version", "3"),
        ])?;
        let response = self.transport.post(&url, body, Some(headers)).await?;
        parse_media_page(&response, kind)
    }
}
fn media_request(
    xuid: &str,
    kind: MediaKind,
    continuation: Option<&str>,
) -> Result<(String, serde_json::Value), WebApiError> {
    if xuid.is_empty() || !xuid.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WebApiError::parse("invalid xuid"));
    }
    let path = match kind {
        MediaKind::Screenshot => "screenshots",
        MediaKind::Clip => "gameclips",
    };
    let mut url = reqwest::Url::parse(&format!("https://mediahub.xboxlive.com/{path}/search"))
        .map_err(|e| WebApiError::parse(e.to_string()))?;
    url.query_pairs_mut().append_pair("maxItems", "50");
    if let Some(token) = continuation {
        url.query_pairs_mut()
            .append_pair("continuationToken", token);
    }
    Ok((
        url.to_string(),
        json!({"query":format!("OwnerXuid eq {xuid}"),"max":50}),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_search_body_on_continuation() {
        let (url, body) = media_request("2533274799", MediaKind::Clip, Some("next/+")).unwrap();
        assert!(url.contains("gameclips/search?"));
        assert!(url.contains("continuationToken=next%2F%2B"));
        assert_eq!(body["query"], "OwnerXuid eq 2533274799");
        assert!(media_request("123 or 1", MediaKind::Screenshot, None).is_err());
    }
}
