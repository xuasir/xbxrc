use std::fs::{create_dir_all, File, OpenOptions};
#[cfg(not(test))]
use std::fs::{read_dir, remove_file};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use super::policy::{
    default_trace_metadata, RuntimeTraceProfile, TraceDimension, TraceDimensionSet,
    TraceFileBudget, TraceImportance, TraceMetadata,
};

#[cfg(test)]
use std::sync::OnceLock;

pub type RuntimeTraceRecorderRef = Arc<RuntimeTraceRecorder>;

const TRACE_SCHEMA_VERSION: u32 = 3;
const WRITER_FLUSH_INTERVAL: Duration = Duration::from_millis(40);
const WRITER_BATCH_LIMIT: usize = 256;
const WRITER_PENDING_ROW_LIMIT: u64 = 8_192;
const BUDGET_NOTICE_INTERVAL_MS: u128 = 60_000;
static TRACE_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub enum RuntimeTraceCategory {
    Event,
    Decision,
    State,
    Snapshot,
    Log,
}

impl RuntimeTraceCategory {
    fn as_str(self) -> &'static str {
        match self {
            RuntimeTraceCategory::Event => "event",
            RuntimeTraceCategory::Decision => "decision",
            RuntimeTraceCategory::State => "state",
            RuntimeTraceCategory::Snapshot => "snapshot",
            RuntimeTraceCategory::Log => "log",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "event" => Some(RuntimeTraceCategory::Event),
            "decision" => Some(RuntimeTraceCategory::Decision),
            "state" => Some(RuntimeTraceCategory::State),
            "snapshot" => Some(RuntimeTraceCategory::Snapshot),
            "log" => Some(RuntimeTraceCategory::Log),
            _ => None,
        }
    }
}

struct TraceInner {
    trace_mode: String,
    profile: RuntimeTraceProfile,
    dimensions: TraceDimensionSet,
    budget: TraceFileBudget,
    path: Option<PathBuf>,
    bytes_written: u64,
}

#[derive(Debug, Default)]
struct TraceDropCounters {
    debug_dropped: u64,
    raw_dropped: u64,
    last_notice_at_ms: u128,
}

enum WriterCommand {
    Write(Vec<u8>),
    SwapFile(Option<File>),
    Sync(Sender<()>),
    Shutdown,
}

/// 开发期协助日志：独立写入项目目录，便于后续整体清理。
pub struct RuntimeTraceRecorder {
    inner: Mutex<TraceInner>,
    writer_tx: Sender<WriterCommand>,
    writer_thread: Mutex<Option<JoinHandle<()>>>,
    sequence: AtomicU64,
    queued_rows: Arc<AtomicU64>,
    drop_counters: Mutex<TraceDropCounters>,
}

impl RuntimeTraceRecorder {
    /// `trace_mode` 与设置项 `runtime_trace_mode` 一致；`off` 时不创建文件、不写盘。
    pub fn new_with_mode(trace_mode: &str) -> std::io::Result<Self> {
        Self::new_with_config(trace_mode, None)
    }

    pub fn new_with_config(
        trace_mode: &str,
        dimensions_expression: Option<&str>,
    ) -> std::io::Result<Self> {
        let (writer_tx, writer_rx) = mpsc::channel();
        let queued_rows = Arc::new(AtomicU64::new(0));
        let writer_queued_rows = queued_rows.clone();
        let writer_thread = thread::Builder::new()
            .name("runtime-trace-writer".to_string())
            .spawn(move || writer_loop(writer_rx, writer_queued_rows))
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let profile = RuntimeTraceProfile::from_stored_mode(trace_mode);
        let dimensions = TraceDimensionSet::effective_for_profile(profile, dimensions_expression);
        let budget = profile.budget();
        let recorder = Self {
            inner: Mutex::new(TraceInner {
                trace_mode: profile.as_str().to_string(),
                profile,
                dimensions,
                budget,
                path: None,
                bytes_written: 0,
            }),
            writer_tx,
            writer_thread: Mutex::new(Some(writer_thread)),
            sequence: AtomicU64::new(0),
            queued_rows,
            drop_counters: Mutex::new(TraceDropCounters::default()),
        };
        if profile != RuntimeTraceProfile::Off {
            recorder.open_trace_output(profile, dimensions, budget, "initial")?;
        }
        Ok(recorder)
    }

    fn open_trace_output(
        &self,
        profile: RuntimeTraceProfile,
        dimensions: TraceDimensionSet,
        budget: TraceFileBudget,
        reason: &str,
    ) -> std::io::Result<()> {
        let root = trace_root_dir();
        create_dir_all(&root)?;
        let path = next_trace_path(&root);
        let path_open_payload = path.display().to_string();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.swap_writer_file(Some(file))?;
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            inner.path = Some(path.clone());
            inner.trace_mode = profile.as_str().to_string();
            inner.profile = profile;
            inner.dimensions = dimensions;
            inner.budget = budget;
            inner.bytes_written = 0;
        }
        prune_trace_files(&root, budget.max_files, &path);
        self.record_state(
            "trace",
            "fileOpened",
            None,
            json!({
                "path": path_open_payload,
                "reason": reason,
                "traceProfile": profile.as_str(),
                "dimensions": dimensions.names(),
                "budget": {
                    "maxFileBytes": budget.max_file_bytes,
                    "maxFiles": budget.max_files,
                },
            }),
        );
        Ok(())
    }

    /// 设置页修改 `runtime_trace_mode` 后立即应用：开关盘写入、更新当前模式字段。
    pub fn apply_trace_mode(&self, mode: &str) -> std::io::Result<()> {
        self.apply_trace_config(mode, None)
    }

    pub fn apply_trace_config(
        &self,
        mode: &str,
        dimensions_expression: Option<&str>,
    ) -> std::io::Result<()> {
        let profile = RuntimeTraceProfile::from_stored_mode(mode);
        let dimensions = TraceDimensionSet::effective_for_profile(profile, dimensions_expression);
        let budget = profile.budget();
        {
            let inner = self
                .inner
                .lock()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            if inner.profile == profile && inner.dimensions == dimensions {
                return Ok(());
            }
        }
        if profile == RuntimeTraceProfile::Off {
            self.swap_writer_file(None)?;
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            inner.trace_mode = profile.as_str().to_string();
            inner.profile = profile;
            inner.dimensions = dimensions;
            inner.budget = budget;
            inner.path = None;
            inner.bytes_written = 0;
            return Ok(());
        }

        self.open_trace_output(profile, dimensions, budget, "traceConfigChanged")
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.sync_writer();
        self.inner.lock().ok().and_then(|g| g.path.clone())
    }

    pub fn trace_mode(&self) -> String {
        self.inner
            .lock()
            .map(|g| g.trace_mode.clone())
            .unwrap_or_default()
    }

    pub fn trace_profile(&self) -> RuntimeTraceProfile {
        self.inner
            .lock()
            .map(|g| g.profile)
            .unwrap_or(RuntimeTraceProfile::Off)
    }

    pub fn trace_dimensions(&self) -> TraceDimensionSet {
        self.inner
            .lock()
            .map(|g| g.dimensions)
            .unwrap_or_else(|_| TraceDimensionSet::empty())
    }

    pub fn disk_enabled(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .map(|g| g.path.is_some())
            .unwrap_or(false)
    }

    pub fn record<T: Serialize>(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
    ) {
        let payload = match serde_json::to_value(payload) {
            Ok(value) => value,
            Err(error) => json!({
                "serializeError": error.to_string(),
            }),
        };
        self.record_value_with_category(
            RuntimeTraceCategory::Event,
            domain,
            event,
            session_id,
            payload,
        );
    }

    pub fn record_event<T: Serialize>(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
    ) {
        self.record_with_category(
            RuntimeTraceCategory::Event,
            domain,
            event,
            session_id,
            payload,
        );
    }

    pub fn record_decision<T: Serialize>(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
    ) {
        self.record_with_category(
            RuntimeTraceCategory::Decision,
            domain,
            event,
            session_id,
            payload,
        );
    }

    pub fn record_state<T: Serialize>(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
    ) {
        self.record_with_category(
            RuntimeTraceCategory::State,
            domain,
            event,
            session_id,
            payload,
        );
    }

    pub fn record_snapshot<T: Serialize>(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
    ) {
        self.record_with_category(
            RuntimeTraceCategory::Snapshot,
            domain,
            event,
            session_id,
            payload,
        );
    }

    pub fn record_log<T: Serialize>(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
    ) {
        self.record_with_category(
            RuntimeTraceCategory::Log,
            domain,
            event,
            session_id,
            payload,
        );
    }

    pub fn record_with_category<T: Serialize>(
        &self,
        category: RuntimeTraceCategory,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
    ) {
        let payload = match serde_json::to_value(payload) {
            Ok(value) => value,
            Err(error) => json!({
                "serializeError": error.to_string(),
            }),
        };
        self.record_value_with_category(category, domain, event, session_id, payload);
    }

    pub fn record_with_metadata<T: Serialize>(
        &self,
        category: RuntimeTraceCategory,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: T,
        metadata: TraceMetadata,
    ) {
        let payload = match serde_json::to_value(payload) {
            Ok(value) => value,
            Err(error) => json!({
                "serializeError": error.to_string(),
            }),
        };
        self.record_value_with_metadata(category, domain, event, session_id, payload, metadata);
    }

    pub fn record_value(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: Value,
    ) {
        self.record_value_with_category(
            RuntimeTraceCategory::Event,
            domain,
            event,
            session_id,
            payload,
        );
    }

    pub fn record_value_with_category(
        &self,
        category: RuntimeTraceCategory,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: Value,
    ) {
        let metadata = default_trace_metadata(category.as_str(), domain, event, &payload);
        self.record_value_with_metadata(category, domain, event, session_id, payload, metadata);
    }

    pub fn record_value_with_metadata(
        &self,
        category: RuntimeTraceCategory,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: Value,
        metadata: TraceMetadata,
    ) {
        let now = now_ms();
        let (trace_mode, profile, dimensions, budget, bytes_written) = match self.inner.lock() {
            Ok(inner) => {
                if inner.path.is_none() {
                    return;
                }
                (
                    inner.trace_mode.clone(),
                    inner.profile,
                    inner.dimensions,
                    inner.budget,
                    inner.bytes_written,
                )
            }
            Err(_) => return,
        };
        if !super::policy::should_record_trace_event(
            profile, dimensions, metadata, domain, event, now as f64,
        ) {
            return;
        }
        if self.should_drop_for_queue_pressure(metadata) {
            self.note_dropped_trace_row(metadata);
            return;
        }
        let line = json!({
            "schemaVersion": TRACE_SCHEMA_VERSION,
            "seq": self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            "tsMs": now,
            "traceMode": trace_mode,
            "traceProfile": profile.as_str(),
            "dimension": metadata.dimension.as_str(),
            "importance": metadata.importance.as_str(),
            "category": category.as_str(),
            "domain": domain,
            "event": event,
            "sessionId": session_id,
            "payload": payload,
        });
        let mut bytes = match serde_json::to_vec(&line) {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        bytes.push(b'\n');
        if budget.max_file_bytes > 0
            && bytes_written > 0
            && bytes_written.saturating_add(bytes.len() as u64) > budget.max_file_bytes
        {
            let _ = self.open_trace_output(profile, dimensions, budget, "budgetRotate");
        }
        self.enqueue_trace_bytes(bytes);
    }

    fn should_drop_for_queue_pressure(&self, metadata: TraceMetadata) -> bool {
        if metadata.importance <= TraceImportance::Key {
            return false;
        }
        self.queued_rows.load(Ordering::Relaxed) >= WRITER_PENDING_ROW_LIMIT
    }

    fn note_dropped_trace_row(&self, metadata: TraceMetadata) {
        let mut should_emit_notice = None;
        let now = now_ms();
        if let Ok(mut counters) = self.drop_counters.lock() {
            match metadata.importance {
                TraceImportance::Raw => counters.raw_dropped += 1,
                TraceImportance::Debug => counters.debug_dropped += 1,
                _ => {}
            }
            if counters.last_notice_at_ms == 0 {
                counters.last_notice_at_ms = now;
            }
            if now.saturating_sub(counters.last_notice_at_ms) >= BUDGET_NOTICE_INTERVAL_MS {
                should_emit_notice = Some((counters.debug_dropped, counters.raw_dropped));
                counters.debug_dropped = 0;
                counters.raw_dropped = 0;
                counters.last_notice_at_ms = now;
            }
        }
        if let Some((debug_dropped, raw_dropped)) = should_emit_notice {
            self.record_with_metadata(
                RuntimeTraceCategory::State,
                "trace",
                "traceBudgetNotice",
                None,
                json!({
                    "reason": "writerQueuePressure",
                    "debugDropped": debug_dropped,
                    "rawDropped": raw_dropped,
                    "pendingRows": self.queued_rows.load(Ordering::Relaxed),
                }),
                TraceMetadata::new(TraceDimension::Core, TraceImportance::Essential),
            );
        }
    }

    fn enqueue_trace_bytes(&self, bytes: Vec<u8>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.bytes_written = inner.bytes_written.saturating_add(bytes.len() as u64);
        }
        self.queued_rows.fetch_add(1, Ordering::Relaxed);
        if self.writer_tx.send(WriterCommand::Write(bytes)).is_err() {
            self.queued_rows.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn swap_writer_file(&self, file: Option<File>) -> std::io::Result<()> {
        self.writer_tx
            .send(WriterCommand::SwapFile(file))
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "runtime trace writer stopped",
                )
            })
    }

    fn sync_writer(&self) {
        let (tx, rx) = mpsc::channel();
        if self.writer_tx.send(WriterCommand::Sync(tx)).is_err() {
            return;
        }
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

impl Drop for RuntimeTraceRecorder {
    fn drop(&mut self) {
        let _ = self.writer_tx.send(WriterCommand::Shutdown);
        if let Ok(mut handle) = self.writer_thread.lock() {
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn writer_loop(rx: Receiver<WriterCommand>, queued_rows: Arc<AtomicU64>) {
    let mut file: Option<File> = None;
    let mut wrote_since_flush = false;
    loop {
        match rx.recv_timeout(WRITER_FLUSH_INTERVAL) {
            Ok(command) => {
                if !handle_writer_command(
                    command,
                    &rx,
                    &mut file,
                    &mut wrote_since_flush,
                    &queued_rows,
                ) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => flush_writer(&mut file, &mut wrote_since_flush),
            Err(RecvTimeoutError::Disconnected) => {
                flush_writer(&mut file, &mut wrote_since_flush);
                break;
            }
        }
    }
}

fn handle_writer_command(
    command: WriterCommand,
    rx: &Receiver<WriterCommand>,
    file: &mut Option<File>,
    wrote_since_flush: &mut bool,
    queued_rows: &AtomicU64,
) -> bool {
    match command {
        WriterCommand::Write(bytes) => {
            write_bytes(file, wrote_since_flush, &bytes);
            queued_rows.fetch_sub(1, Ordering::Relaxed);
            for _ in 0..WRITER_BATCH_LIMIT {
                match rx.try_recv() {
                    Ok(WriterCommand::Write(bytes)) => {
                        write_bytes(file, wrote_since_flush, &bytes);
                        queued_rows.fetch_sub(1, Ordering::Relaxed);
                    }
                    Ok(other) => {
                        return handle_writer_command(
                            other,
                            rx,
                            file,
                            wrote_since_flush,
                            queued_rows,
                        )
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        flush_writer(file, wrote_since_flush);
                        return false;
                    }
                }
            }
        }
        WriterCommand::SwapFile(next_file) => {
            flush_writer(file, wrote_since_flush);
            *file = next_file;
        }
        WriterCommand::Sync(done) => {
            flush_writer(file, wrote_since_flush);
            let _ = done.send(());
        }
        WriterCommand::Shutdown => {
            flush_writer(file, wrote_since_flush);
            return false;
        }
    }
    true
}

fn write_bytes(file: &mut Option<File>, wrote_since_flush: &mut bool, bytes: &[u8]) {
    let Some(file) = file.as_mut() else {
        return;
    };
    if file.write_all(bytes).is_ok() {
        *wrote_since_flush = true;
    }
}

fn flush_writer(file: &mut Option<File>, wrote_since_flush: &mut bool) {
    if !*wrote_since_flush {
        return;
    }
    if let Some(file) = file.as_mut() {
        let _ = file.flush();
    }
    *wrote_since_flush = false;
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

fn trace_root_dir() -> PathBuf {
    #[cfg(test)]
    {
        current_test_trace_run_dir()
    }
    #[cfg(not(test))]
    {
        project_root().join("runtime-logs")
    }
}

#[cfg(test)]
fn current_test_trace_run_dir() -> PathBuf {
    static TEST_TRACE_RUN_DIR: OnceLock<PathBuf> = OnceLock::new();
    TEST_TRACE_RUN_DIR
        .get_or_init(|| {
            initialize_test_trace_run_dir(&project_root().join("target/runtime-logs-tests"))
        })
        .clone()
}

#[cfg(test)]
fn initialize_test_trace_run_dir(root: &Path) -> PathBuf {
    let _ = std::fs::remove_dir_all(root);
    let run_dir = root.join(format!("run-{}-pid{}", now_ms(), std::process::id()));
    let _ = create_dir_all(&run_dir);
    run_dir
}

fn next_trace_path(root: &Path) -> PathBuf {
    let file_id = TRACE_FILE_ID.fetch_add(1, Ordering::Relaxed) + 1;
    root.join(format!("runtime-trace-{}-{}.jsonl", now_ms(), file_id))
}

fn prune_trace_files(root: &Path, max_files: usize, active_path: &Path) {
    #[cfg(test)]
    {
        let _ = (root, max_files, active_path);
        return;
    }
    #[cfg(not(test))]
    {
        if max_files == 0 {
            return;
        }
        let Ok(entries) = read_dir(root) else {
            return;
        };
        let mut traces: Vec<(PathBuf, SystemTime)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?;
                if !name.starts_with("runtime-trace-") || !name.ends_with(".jsonl") {
                    return None;
                }
                if path == active_path {
                    return None;
                }
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((path, modified))
            })
            .collect();
        let inactive_keep = max_files.saturating_sub(1);
        if traces.len() <= inactive_keep {
            return;
        }
        traces.sort_by_key(|(_, modified)| *modified);
        let remove_count = traces.len().saturating_sub(inactive_keep);
        for (path, _) in traces.into_iter().take(remove_count) {
            let _ = remove_file(path);
        }
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn read_trace_lines(recorder: &RuntimeTraceRecorder) -> Vec<Value> {
        let path = recorder.path().expect("trace path");
        let contents = fs::read_to_string(&path).expect("trace contents");
        contents
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect()
    }

    #[test]
    fn records_lines_via_background_writer() {
        let recorder = RuntimeTraceRecorder::new_with_mode("verbose").expect("trace recorder");
        recorder.record_event("test", "alpha", Some("session-1"), json!({ "value": 1 }));
        recorder.record_state("test", "beta", None, json!({ "value": 2 }));

        let entries = read_trace_lines(&recorder);
        assert!(entries.iter().any(|entry| entry["event"] == "fileOpened"));
        assert!(entries.iter().any(|entry| entry["event"] == "alpha"));
        assert!(entries.iter().any(|entry| entry["event"] == "beta"));
        let alpha = entries
            .iter()
            .find(|entry| entry["event"] == "alpha")
            .expect("alpha trace row");
        assert_eq!(alpha["schemaVersion"], TRACE_SCHEMA_VERSION);
        assert_eq!(alpha["traceProfile"], "dev");
        assert_eq!(alpha["dimension"], "core");
        assert_eq!(alpha["importance"], "key");
    }

    #[test]
    fn apply_trace_mode_rotates_output_file() {
        let recorder = RuntimeTraceRecorder::new_with_mode("minimal").expect("trace recorder");
        let first_path = recorder.path().expect("first path");
        recorder.record_event("test", "beforeRotate", None, json!({}));

        recorder
            .apply_trace_mode("verbose")
            .expect("rotate trace mode");
        recorder.record_event("test", "afterRotate", None, json!({}));

        let second_path = recorder.path().expect("second path");
        assert_ne!(first_path, second_path);

        let first_contents = fs::read_to_string(&first_path).expect("first trace contents");
        assert!(first_contents.contains("beforeRotate"));

        let second_contents = fs::read_to_string(&second_path).expect("second trace contents");
        assert!(second_contents.contains("afterRotate"));
        assert!(second_contents.contains("traceConfigChanged"));
    }

    #[test]
    fn production_profile_filters_debug_rows() {
        let recorder = RuntimeTraceRecorder::new_with_mode("production").expect("trace recorder");
        recorder.record_with_metadata(
            RuntimeTraceCategory::Event,
            "xbxengine",
            "runtimeEventRaw",
            None,
            json!({ "value": 1 }),
            TraceMetadata::new(TraceDimension::Recovery, TraceImportance::Debug),
        );
        recorder.record_with_metadata(
            RuntimeTraceCategory::Decision,
            "xbxengine",
            "recoveryDecisionLedger",
            None,
            json!({ "value": 2 }),
            TraceMetadata::new(TraceDimension::Recovery, TraceImportance::Key),
        );

        let entries = read_trace_lines(&recorder);
        assert!(!entries
            .iter()
            .any(|entry| entry["event"] == "runtimeEventRaw"));
        assert!(entries
            .iter()
            .any(|entry| entry["event"] == "recoveryDecisionLedger"));
    }

    #[test]
    fn file_budget_rotates_output_file() {
        let recorder = RuntimeTraceRecorder::new_with_mode("dev").expect("trace recorder");
        let first_path = recorder.path().expect("first path");
        {
            let mut inner = recorder.inner.lock().expect("trace inner");
            inner.budget.max_file_bytes = 512;
        }

        recorder.record_event(
            "xbxengine",
            "largeTraceRow",
            None,
            json!({ "blob": "x".repeat(1024) }),
        );

        let second_path = recorder.path().expect("second path");
        assert_ne!(first_path, second_path);
        let second_contents = fs::read_to_string(&second_path).expect("second trace contents");
        assert!(second_contents.contains("budgetRotate"));
        assert!(second_contents.contains("largeTraceRow"));
    }

    #[test]
    fn off_mode_disables_disk_output() {
        let recorder = RuntimeTraceRecorder::new_with_mode("off").expect("trace recorder");
        recorder.record_event("test", "ignored", None, json!({}));

        assert!(!recorder.disk_enabled());
        assert!(recorder.path().is_none());
    }

    #[test]
    fn initializes_new_test_run_by_clearing_previous_output() {
        let root = project_root().join("target/runtime-logs-tests-reset-check");
        fs::create_dir_all(root.join("old-run")).expect("create old run dir");
        fs::write(root.join("old-run/stale.jsonl"), "{}\n").expect("seed stale trace");

        let run_dir = initialize_test_trace_run_dir(&root);

        let stale_path = root.join("old-run/stale.jsonl");
        assert!(!stale_path.exists());
        assert!(run_dir.exists());
        assert_eq!(run_dir.parent(), Some(root.as_path()));
    }
}
