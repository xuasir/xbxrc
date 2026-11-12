use super::*;

#[tokio::test]
async fn repeated_detail_queries_share_one_slot_and_committed_value() {
    let mut cache = DetailSlots::new(30);
    let a = cache.slot("US:zh-CN:A".into()).unwrap();
    let b = cache.slot("US:zh-CN:A".into()).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    *a.lock().await = Some(42);
    assert_eq!(*b.lock().await, Some(42));
}

#[test]
fn lru_preserves_active_queries_and_bounds_thirty_browsed_details() {
    let mut cache = DetailSlots::<u64>::new(30);
    let active = cache.slot("active".into()).unwrap();
    for index in 0..80 {
        cache.slot(format!("detail:{index}"));
    }
    assert_eq!(cache.entries.len(), 30);
    assert!(Arc::ptr_eq(&active, &cache.slot("active".into()).unwrap()));
    assert!(!cache.entries.contains_key("detail:1"));
}

#[test]
fn market_and_language_are_independent_and_full_inflight_capacity_is_bounded() {
    let mut cache = DetailSlots::<u64>::new(2);
    let a = cache.slot("US:zh-CN:A".into()).unwrap();
    let b = cache.slot("US:en-US:A".into()).unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(cache.slot("GB:en-US:A".into()).is_none());
    drop(a);
    assert!(cache.slot("GB:en-US:A".into()).is_some());
    assert_eq!(cache.entries.len(), 2);
}
