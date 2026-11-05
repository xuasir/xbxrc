use super::*;

#[test]
fn reserves_priority_rows_and_releases_inflight_budget() {
    let budget = Arc::new(QueueBudget::default());
    let mut retained = Vec::new();
    for _ in 0..6_144 {
        retained.push(budget.reserve(1, TraceImportance::Raw).unwrap());
    }
    assert!(budget.reserve(1, TraceImportance::Debug).is_none());
    for _ in 6_144..7_936 {
        retained.push(budget.reserve(1, TraceImportance::Key).unwrap());
    }
    assert!(budget.reserve(1, TraceImportance::Key).is_none());
    for _ in 7_936..QUEUE_MAX_ROWS {
        retained.push(budget.reserve(1, TraceImportance::Essential).unwrap());
    }
    assert!(budget.reserve(1, TraceImportance::Essential).is_none());
    assert_eq!(budget.snapshot().rows, QUEUE_MAX_ROWS);
    drop(retained);
    assert_eq!(budget.snapshot().rows, 0);
    assert_eq!(budget.snapshot().bytes, 0);
}

#[test]
fn enforces_byte_reserve_and_serialized_row_limit() {
    let budget = Arc::new(QueueBudget::default());
    let mut retained = Vec::new();
    for _ in 0..96 {
        retained.push(
            budget
                .reserve(MAX_ROW_BYTES, TraceImportance::Debug)
                .unwrap(),
        );
    }
    assert!(budget.reserve(1, TraceImportance::Raw).is_none());
    for _ in 96..124 {
        retained.push(budget.reserve(MAX_ROW_BYTES, TraceImportance::Key).unwrap());
    }
    assert!(budget.reserve(1, TraceImportance::Key).is_none());
    for _ in 124..128 {
        retained.push(
            budget
                .reserve(MAX_ROW_BYTES, TraceImportance::Essential)
                .unwrap(),
        );
    }
    assert_eq!(budget.snapshot().bytes, QUEUE_MAX_BYTES);
    assert!(budget.reserve(1, TraceImportance::Essential).is_none());
    assert!(budget
        .reserve(MAX_ROW_BYTES + 1, TraceImportance::Essential)
        .is_none());
    assert!(encode_row(&serde_json::json!({ "blob": "x".repeat(MAX_ROW_BYTES) })).is_err());
}

#[test]
fn concurrent_producers_share_one_atomic_row_and_byte_budget() {
    let budget = Arc::new(QueueBudget::default());
    let producers: Vec<_> = (0..8)
        .map(|_| {
            let budget = budget.clone();
            std::thread::spawn(move || {
                (0..2_000)
                    .filter_map(|_| budget.reserve(8192, TraceImportance::Essential))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let retained: Vec<_> = producers
        .into_iter()
        .map(|producer| producer.join().unwrap())
        .collect();
    let usage = budget.snapshot();
    assert!(usage.peak_rows <= QUEUE_MAX_ROWS);
    assert_eq!(usage.peak_bytes, QUEUE_MAX_BYTES);
    assert_eq!(usage.bytes, QUEUE_MAX_BYTES);
    drop(retained);
    assert_eq!(budget.snapshot().bytes, 0);
}

#[test]
fn uneven_serialization_chunks_keep_allocated_capacity_inside_row_budget() {
    let row = encode_row(&serde_json::json!({
        "first": "x".repeat(40_000),
        "second": "x".repeat(23_000),
    }))
    .unwrap();
    assert!(row.len() + SEQUENCE_OVERHEAD <= MAX_ROW_BYTES);
    assert!(row.capacity() <= MAX_ROW_BYTES);
    let budget = Arc::new(QueueBudget::default());
    let reservation = budget
        .reserve(row.capacity(), TraceImportance::Essential)
        .unwrap();
    assert_eq!(budget.snapshot().bytes, row.capacity());
    drop(reservation);
    assert_eq!(budget.snapshot().bytes, 0);
}

#[test]
fn concurrent_drop_snapshots_keep_importance_and_reason_counts_paired() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let counters = Arc::new(DropCounters::default());
    let active = Arc::new(AtomicUsize::new(4));
    let producers: Vec<_> = (0..4)
        .map(|_| {
            let counters = counters.clone();
            let active = active.clone();
            std::thread::spawn(move || {
                for _ in 0..10_000 {
                    counters.note(TraceImportance::Essential, DropReason::Oversized);
                }
                active.fetch_sub(1, Ordering::Release);
            })
        })
        .collect();
    let mut total = 0;
    while active.load(Ordering::Acquire) > 0 {
        let snapshot = counters.take();
        assert_eq!(snapshot.essential, snapshot.oversized);
        total += snapshot.total();
        std::thread::yield_now();
    }
    for producer in producers {
        producer.join().unwrap();
    }
    let pending = counters.take();
    assert_eq!(pending.essential, pending.oversized);
    counters.restore(pending);
    let restored = counters.take();
    assert_eq!(restored.essential, restored.oversized);
    assert_eq!(total + restored.total(), 40_000);
}
