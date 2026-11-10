//! 有界跨区域发现；目标查找与列表查找共用控制面遍历。
use crate::policy::types::Region;
use serde_json::Value;
use std::future::Future;
use xbox_webapi::WebApiError;

pub fn console_matches(value: &Value, target: &str) -> bool {
    ["serverId", "id", "deviceId"]
        .iter()
        .any(|key| value.get(key).and_then(Value::as_str) == Some(target))
}

pub fn ordered_home_regions(regions: &[Region], cached: Option<&str>) -> Vec<Region> {
    let mut ordered = regions.to_vec();
    ordered.sort_by_key(|region| {
        if cached.is_some_and(|name| name.eq_ignore_ascii_case(&region.name)) {
            0
        } else if region.is_default {
            1
        } else {
            2
        }
    });
    ordered
}

pub async fn discover_home<F, Fut, C>(
    regions: &[Region],
    target: Option<&str>,
    mut request: F,
    current: C,
) -> Result<Option<(Region, Vec<Value>)>, WebApiError>
where
    F: FnMut(Region) -> Fut,
    Fut: Future<Output = Result<Vec<Value>, WebApiError>>,
    C: Fn() -> bool,
{
    let work = async {
        let mut succeeded = false;
        let mut last_error = None;
        for region in regions {
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(5), request(region.clone()))
                    .await;
            match result {
                Ok(Ok(items)) => {
                    succeeded = true;
                    if target.map_or(!items.is_empty(), |target| {
                        items.iter().any(|item| console_matches(item, target))
                    }) {
                        return Ok(Some((region.clone(), items)));
                    }
                }
                Ok(Err(error)) if matches!(error.to_status_code(), Some(401 | 403)) => {
                    return Err(error)
                }
                Ok(Err(error)) => last_error = Some(error),
                Err(_) => last_error = Some(WebApiError::parse("主机区域查询超时，请重试")),
            }
        }
        if succeeded {
            Ok(None)
        } else {
            Err(last_error.unwrap_or_else(|| WebApiError::parse("主机可用区域暂不可用")))
        }
    };
    let revoked = async {
        loop {
            if !current() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    tokio::select! {
        biased;
        _ = revoked => Err(WebApiError::parse("登录或连接设置已更新，请重新查询")),
        result = tokio::time::timeout(std::time::Duration::from_secs(30), work) => result.unwrap_or_else(|_| Err(WebApiError::parse("主机跨区域查询超时，请重试"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn regions() -> Vec<Region> {
        vec![
            Region {
                name: "first".into(),
                is_default: true,
                ..Default::default()
            },
            Region {
                name: "second".into(),
                ..Default::default()
            },
        ]
    }
    #[tokio::test]
    async fn target_search_continues_past_other_consoles() {
        let result = discover_home(
            &regions(),
            Some("target"),
            |region| async move {
                Ok(vec![
                    json!({"serverId": if region.is_default { "other" } else { "target" }}),
                ])
            },
            || true,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.0.name, "second");
    }
    #[tokio::test]
    async fn empty_and_network_failure_continue_but_auth_returns() {
        let result = discover_home(
            &regions(),
            None,
            |region| async move {
                if region.is_default {
                    Err(WebApiError::parse("network"))
                } else {
                    Ok(vec![json!({"id":"console"})])
                }
            },
            || true,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.0.name, "second");
        assert!(
            discover_home(&regions(), None, |_| async { Ok(vec![]) }, || true)
                .await
                .unwrap()
                .is_none()
        );
        assert!(discover_home(
            &regions(),
            None,
            |_| async { Ok(vec![json!({"id":"console"})]) },
            || false
        )
        .await
        .is_err());
    }
    #[tokio::test(start_paused = true)]
    async fn per_region_timeout_allows_next_region() {
        let result = discover_home(
            &regions(),
            None,
            |region| async move {
                if region.is_default {
                    std::future::pending::<()>().await;
                }
                Ok(vec![json!({"id":"console"})])
            },
            || true,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.0.name, "second");
    }
    #[tokio::test(start_paused = true)]
    async fn total_budget_revocation_and_auth_stop_the_scan() {
        let mut many = regions();
        many.extend((0..6).map(|index| Region {
            name: format!("extra-{index}"),
            ..Default::default()
        }));
        let start = tokio::time::Instant::now();
        assert!(discover_home(
            &many,
            None,
            |_| async { std::future::pending::<Result<Vec<Value>, WebApiError>>().await },
            || true
        )
        .await
        .is_err());
        assert_eq!(start.elapsed(), std::time::Duration::from_secs(30));
        for status in [401, 403] {
            let calls = std::sync::atomic::AtomicUsize::new(0);
            let error = discover_home(
                &regions(),
                None,
                |_| {
                    calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    async move { Err(WebApiError::http(status, "rejected")) }
                },
                || true,
            )
            .await
            .unwrap_err();
            assert_eq!(error.to_status_code(), Some(status));
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
        let start = tokio::time::Instant::now();
        assert!(discover_home(
            &regions(),
            None,
            |_| async { std::future::pending::<Result<Vec<Value>, WebApiError>>().await },
            || start.elapsed() < std::time::Duration::from_millis(125)
        )
        .await
        .is_err());
        assert!(start.elapsed() <= std::time::Duration::from_millis(150));
        assert_eq!(
            ordered_home_regions(&regions(), Some("second"))[0].name,
            "second"
        );
        assert_eq!(
            ordered_home_regions(&regions(), Some("expired"))[0].name,
            "first"
        );
    }
}
