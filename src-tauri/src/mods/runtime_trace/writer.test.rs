use super::*;

fn root() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("xbxrc-trace-writer-")
        .tempdir()
        .unwrap();
    let root = directory.path().join("writer");
    (directory, root)
}

fn config(generation: u64, max_file_bytes: u64) -> OutputConfig {
    OutputConfig {
        profile: RuntimeTraceProfile::Dev,
        dimensions: TraceDimensionSet::default_for_profile(RuntimeTraceProfile::Dev),
        budget: TraceFileBudget {
            max_file_bytes,
            max_files: 10,
        },
        generation,
    }
}

fn rows(root: &Path) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    for entry in read_dir(root).unwrap().flatten() {
        let contents = std::fs::read_to_string(entry.path()).unwrap();
        let values: Vec<serde_json::Value> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let sequence: Vec<_> = values
            .iter()
            .map(|value| value["seq"].as_u64().unwrap())
            .collect();
        assert!(sequence.windows(2).all(|pair| pair[0] < pair[1]));
        rows.extend(values);
    }
    rows.sort_by_key(|row| row["seq"].as_u64().unwrap());
    rows
}

#[test]
fn stalled_writer_admission_preserves_priority_reserves_and_never_exceeds_hard_budget() {
    let (tx, rx) = mpsc::sync_channel(QUEUE_MAX_ROWS + CONTROL_CAPACITY);
    let writer = WriterHandle {
        tx,
        thread: Mutex::new(None),
        queue: Arc::new(QueueBudget::default()),
        drops: Arc::new(DropCounters::default()),
        path: Arc::new(Mutex::new(None)),
    };
    for _ in 0..6_144 {
        writer.try_write(b"{}\n".to_vec(), 1, TraceImportance::Raw);
    }
    writer.try_write(b"{}\n".to_vec(), 1, TraceImportance::Raw);
    for _ in 6_144..7_936 {
        writer.try_write(b"{}\n".to_vec(), 1, TraceImportance::Key);
    }
    writer.try_write(b"{}\n".to_vec(), 1, TraceImportance::Key);
    for _ in 7_936..QUEUE_MAX_ROWS {
        writer.try_write(b"{}\n".to_vec(), 1, TraceImportance::Essential);
    }
    writer.try_write(b"{}\n".to_vec(), 1, TraceImportance::Essential);
    assert_eq!(writer.usage().rows, QUEUE_MAX_ROWS);
    assert!(writer.usage().bytes <= QUEUE_MAX_BYTES);
    let dropped = writer.drops.take();
    assert_eq!((dropped.raw, dropped.key, dropped.essential), (1, 1, 1));
    for _ in 0..QUEUE_MAX_ROWS {
        drop(rx.try_recv().unwrap());
    }
    assert_eq!(writer.usage().rows, 0);
    assert_eq!(writer.usage().bytes, 0);
}

#[test]
fn concurrent_rows_rotate_in_writer_order_and_shutdown_flushes_every_accepted_row() {
    let (_directory, root) = root();
    let writer = Arc::new(WriterHandle::new(root.clone(), "concurrent-test".to_string()).unwrap());
    assert!(!root.exists());
    writer
        .configure(config(1, 4096), "initial")
        .unwrap()
        .recv()
        .unwrap()
        .unwrap();
    let producers: Vec<_> = (0..4).map(|producer| {
        let writer = writer.clone();
        thread::spawn(move || {
            for index in 0..25 {
                let row = encode_row(&json!({ "event": "concurrent", "payload": { "producer": producer, "index": index, "blob": "x".repeat(500) } })).unwrap();
                writer.try_write(row, 1, TraceImportance::Key);
            }
        })
    }).collect();
    for producer in producers {
        producer.join().unwrap();
    }
    drop(writer);
    let entries = rows(&root);
    assert_eq!(
        entries
            .iter()
            .filter(|row| row["event"] == "concurrent")
            .count(),
        100
    );
    assert!(entries
        .windows(2)
        .all(|pair| pair[0]["seq"].as_u64().unwrap() + 1 == pair[1]["seq"].as_u64().unwrap()));
    assert!(entries
        .iter()
        .filter(|row| row["event"] == "fileOpened")
        .all(|row| row["payload"]["writerThread"] == "runtime-trace-writer"));
    for entry in read_dir(&root).unwrap().flatten() {
        assert!(entry.metadata().unwrap().len() <= 4096);
    }
}

#[test]
fn failed_configuration_keeps_old_output_and_reports_mismatched_generation() {
    let (_directory, root) = root();
    let writer = WriterHandle::new(root.clone(), "config-test".to_string()).unwrap();
    writer
        .configure(config(1, 4096), "initial")
        .unwrap()
        .recv()
        .unwrap()
        .unwrap();
    let first = writer.path().unwrap();
    assert!(writer
        .configure(config(2, 1), "invalidBudget")
        .unwrap()
        .recv()
        .unwrap()
        .is_err());
    assert_eq!(writer.path().unwrap(), first);
    writer.try_write(
        encode_row(&json!({ "event": "wrongGeneration" })).unwrap(),
        2,
        TraceImportance::Essential,
    );
    writer.try_write(
        encode_row(&json!({ "event": "correctGeneration" })).unwrap(),
        1,
        TraceImportance::Key,
    );
    writer.sync();
    let entries = rows(&root);
    assert!(!entries.iter().any(|row| row["event"] == "wrongGeneration"));
    assert!(entries
        .iter()
        .any(|row| row["event"] == "correctGeneration"));
    let notice = entries
        .iter()
        .find(|row| row["event"] == "traceBudgetNotice")
        .unwrap();
    assert_eq!(notice["payload"]["essentialDropped"], 1);
    assert_eq!(notice["payload"]["configurationDropped"], 1);
    drop(writer);
}

#[test]
fn failed_write_conservatively_charges_the_entire_row_to_file_budget() {
    let (_directory, root) = root();
    create_dir_all(&root).unwrap();
    let path = root.join("readonly.jsonl");
    std::fs::write(&path, "").unwrap();
    let file = File::open(&path).unwrap();
    let mut writer = TraceWriter {
        root: root.clone(),
        trace_run_id: "io-failure-test".to_string(),
        queue: Arc::new(QueueBudget::default()),
        drops: Arc::new(DropCounters::default()),
        path: Arc::new(Mutex::new(Some(path))),
        config: Some(config(1, 4096)),
        file: Some(BufWriter::with_capacity(1, file)),
        bytes_written: 0,
        sequence: 0,
        last_notice: Instant::now(),
    };
    let row = encode_row(&json!({ "event": "failedWrite" })).unwrap();
    let attempted = writer.line_size(&row);
    assert!(writer.append(&row).is_err());
    assert_eq!(writer.bytes_written, attempted);
    assert!(writer.bytes_written <= 4096);
    drop(writer);
}

#[test]
fn prune_retains_active_file_and_ignores_unrelated_files() {
    let (_directory, root) = root();
    create_dir_all(&root).unwrap();
    for index in 0..6 {
        std::fs::write(root.join(format!("runtime-trace-1-{index}.jsonl")), "{}\n").unwrap();
    }
    let active = root.join("runtime-trace-1-5.jsonl");
    std::fs::write(root.join("unrelated.txt"), "keep").unwrap();
    prune_files(&root, 2, &active);
    assert!(active.exists());
    assert!(root.join("unrelated.txt").exists());
    assert_eq!(read_dir(&root).unwrap().count(), 3);
}
