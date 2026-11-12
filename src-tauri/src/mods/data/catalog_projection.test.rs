use super::*;
use crate::mods::data::types::DataXcloudCatalogCacheState;

fn title(index: usize) -> DataXcloudTitleSummary {
    serde_json::from_value(serde_json::json!({
        "id": format!("P{index}"), "productId": format!("P{index}"), "titleId": format!("S{index}"),
        "name": format!("Game {index}"), "publisherName": "", "description": "", "tileImageUrl": "", "posterImageUrl": "",
        "categories": [], "supportedInputTypes": [], "hasEntitlement": false, "isRecentlyPlayed": false, "isNew": false
    })).unwrap()
}
fn payload(titles: Vec<DataXcloudTitleSummary>) -> DataXcloudCatalogPayload {
    DataXcloudCatalogPayload {
        content_revision: content_revision(&titles),
        fresh_until: Some(1000),
        scope_key: Some("scope".into()),
        titles,
        cache_state: DataXcloudCatalogCacheState::Fresh,
        updated_at: Some(1),
        refreshing: false,
        sources: Default::default(),
        access: "account".into(),
        error: None,
    }
}

#[test]
fn publication_bounds_chunks_and_chains_versions_with_complete_order() {
    let titles = (0..201).map(title).collect::<Vec<_>>();
    let mut publication = CatalogPublication::new(Vec::new());
    let start = publication.revision.clone();
    let events = publication.publish(payload(titles.clone()), "scope", "refresh");
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].patch.as_ref().unwrap().base_revision, start);
    assert_eq!(
        events[0]
            .patch
            .as_ref()
            .unwrap()
            .order
            .as_ref()
            .unwrap()
            .len(),
        201
    );
    let mut previous = start;
    let mut received = HashMap::new();
    for event in &events {
        assert!(event.titles.is_empty());
        let patch = event.patch.as_ref().unwrap();
        assert!(patch.upserts.len() <= 75);
        assert_eq!(patch.base_revision, previous);
        previous = event.content_revision.clone();
        for title in &patch.upserts {
            received.insert(title.product_id.clone(), title.clone());
        }
    }
    assert_eq!(received.len(), titles.len());
    assert_eq!(previous, content_revision(&titles));
    assert!(events[0].refreshing);
    assert!(!events.last().unwrap().refreshing);
}

#[test]
fn status_and_duplicate_completion_keep_the_content_version() {
    let titles = vec![title(1)];
    let mut publication = CatalogPublication::new(titles.clone());
    let revision = publication.revision.clone();
    let mut status = payload(titles);
    status.error = Some("source unavailable".into());
    status.updated_at = Some(2);
    let event = publication
        .publish(status, "scope", "terminal")
        .pop()
        .unwrap();
    assert_eq!(event.content_revision, revision);
    let patch = event.patch.unwrap();
    assert!(patch.upserts.is_empty());
    assert!(patch.removed_product_ids.is_empty());
    assert!(patch.order.is_none());
    assert_eq!(event.error.as_deref(), Some("source unavailable"));
}

#[test]
fn empty_catalog_removes_every_previous_title_and_commits_empty_order() {
    let mut publication = CatalogPublication::new(vec![title(1), title(2)]);
    let event = publication
        .publish(payload(Vec::new()), "scope", "refresh")
        .pop()
        .unwrap();
    let patch = event.patch.unwrap();
    assert_eq!(patch.removed_product_ids, ["P1", "P2"]);
    assert_eq!(patch.order, Some(Vec::new()));
    assert_eq!(event.content_revision, content_revision(&[]));
}
