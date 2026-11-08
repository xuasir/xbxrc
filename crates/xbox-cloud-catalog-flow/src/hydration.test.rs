use super::{build_overlay_load, hydrate_product_chunks_with_budget, PRODUCT_HYDRATION_CHUNK_SIZE};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn rejects_invalid_mandatory_titles_and_accepts_empty_catalog() {
    for response in [json!({}), json!({"results": null}), json!({"results": {}})] {
        assert!(build_overlay_load(&response, &json!({}), &json!([]), true, true).is_err());
    }
    let load = build_overlay_load(
        &json!({"results": []}),
        &json!({"results": []}),
        &json!([]),
        true,
        true,
    )
    .unwrap();
    assert!(load.product_ids.is_empty());
    assert!(load.entries.is_empty());
}

#[tokio::test]
async fn reports_invalid_products_and_accepts_empty_container() {
    for response in [
        json!({}),
        json!({"Products": null}),
        json!({"Products": "bad"}),
    ] {
        let load = hydrate_product_chunks_with_budget(
            &["9TEST".to_string()],
            Duration::from_secs(1),
            move |_| {
                let response = response.clone();
                async move { Ok(response) }
            },
        )
        .await;
        assert_eq!(load.failed_chunk_count, 1);
        assert_eq!(load.source_failures[0].error["errorKind"], "parse");
    }
    for response in [json!({"Products": []}), json!({"Products": {}})] {
        let load = hydrate_product_chunks_with_budget(
            &["9TEST".to_string()],
            Duration::from_secs(1),
            move |_| {
                let response = response.clone();
                async move { Ok(response) }
            },
        )
        .await;
        assert_eq!(load.failed_chunk_count, 0);
        assert!(load.entries.is_empty());
        assert!(load.source_failures.is_empty());
    }
}

#[tokio::test]
async fn keeps_completed_batches_when_budget_expires_and_cancels_pending_queries() {
    let ids = (0..PRODUCT_HYDRATION_CHUNK_SIZE * 6)
        .map(|index| format!("P{index}"))
        .collect::<Vec<_>>();
    let active = Arc::new(AtomicUsize::new(0));
    let query_active = active.clone();
    let load = hydrate_product_chunks_with_budget(&ids, Duration::from_millis(80), move |chunk| {
        let active = query_active.clone();
        async move {
            let _guard = ActiveQuery::new(active);
            if chunk[0] == "P0" {
                return Ok(json!({"Products": {"P0": {"ProductTitle": "Completed"}}}));
            }
            std::future::pending().await
        }
    })
    .await;
    assert_eq!(load.entries["P0"].name, "Completed");
    assert_eq!(load.failed_chunk_count, 5);
    assert_eq!(load.source_failures[0].error["networkErrorKind"], "timeout");
    tokio::task::yield_now().await;
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bounds_concurrency_and_runs_all_normalized_unique_batches() {
    let mut ids = (0..PRODUCT_HYDRATION_CHUNK_SIZE * 5)
        .map(|index| format!("P{index}"))
        .collect::<Vec<_>>();
    ids.extend([" p0 ".to_string(), " ".to_string()]);
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (query_active, query_maximum, query_calls) =
        (active.clone(), maximum.clone(), calls.clone());
    let load = hydrate_product_chunks_with_budget(&ids, Duration::from_secs(2), move |chunk| {
        let (active, maximum, calls) = (
            query_active.clone(),
            query_maximum.clone(),
            query_calls.clone(),
        );
        async move {
            let _guard = ActiveQuery::new(active.clone());
            maximum.fetch_max(active.load(Ordering::SeqCst), Ordering::SeqCst);
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(chunk.len(), PRODUCT_HYDRATION_CHUNK_SIZE);
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(json!({"Products": [{"ProductId": chunk[0], "ProductTitle": "Batch"}]}))
        }
    })
    .await;
    assert_eq!(maximum.load(Ordering::SeqCst), 4);
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(load.entries.len(), 5);
    assert_eq!(load.failed_chunk_count, 0);
}

struct ActiveQuery(Arc<AtomicUsize>);

impl ActiveQuery {
    fn new(active: Arc<AtomicUsize>) -> Self {
        active.fetch_add(1, Ordering::SeqCst);
        Self(active)
    }
}

impl Drop for ActiveQuery {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
