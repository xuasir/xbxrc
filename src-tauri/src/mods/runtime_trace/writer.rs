use std::fs::{create_dir_all, read_dir, remove_file, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use super::policy::{RuntimeTraceProfile, TraceDimensionSet, TraceFileBudget, TraceImportance};
use super::runtime_state::{
    encode_row, DropCounters, DropReason, QueueBudget, RowReservation, TraceSegment,
    WriterProgress, CONTROL_CAPACITY, MAX_ROW_BYTES, QUEUE_MAX_BYTES, QUEUE_MAX_ROWS,
};
use super::service::{now_ms, TRACE_SCHEMA_VERSION};
use super::types::{TraceFlushStatus, TraceRecordDisposition};

const FLUSH_INTERVAL: Duration = Duration::from_millis(40);
const BATCH_LIMIT: usize = 256;
const NOTICE_INTERVAL: Duration = Duration::from_secs(60);
static TRACE_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub(super) struct OutputConfig {
    pub profile: RuntimeTraceProfile,
    pub dimensions: TraceDimensionSet,
    pub budget: TraceFileBudget,
    pub generation: u64,
}

struct QueuedRow {
    bytes: Vec<u8>,
    generation: u64,
    importance: TraceImportance,
    _reservation: RowReservation,
}

enum WriterCommand {
    Write(QueuedRow),
    Configure(OutputConfig, &'static str, Sender<io::Result<()>>),
    Sync(bool, Sender<io::Result<()>>),
    Shutdown,
}

pub(super) struct WriterHandle {
    tx: SyncSender<WriterCommand>,
    thread: Mutex<Option<JoinHandle<()>>>,
    queue: Arc<QueueBudget>,
    drops: Arc<DropCounters>,
    path: Arc<Mutex<Option<PathBuf>>>,
    progress: Arc<Mutex<WriterProgress>>,
}

impl WriterHandle {
    pub fn new(root: PathBuf, trace_run_id: String) -> io::Result<Self> {
        // payload受行数/字节账本约束，控制指令另外保留固定槽位。
        let (tx, rx) = mpsc::sync_channel(QUEUE_MAX_ROWS + CONTROL_CAPACITY);
        let queue = Arc::new(QueueBudget::default());
        let drops = Arc::new(DropCounters::default());
        let path = Arc::new(Mutex::new(None));
        let progress = Arc::new(Mutex::new(WriterProgress::default()));
        let mut writer = TraceWriter {
            root,
            trace_run_id,
            queue: queue.clone(),
            drops: drops.clone(),
            path: path.clone(),
            progress: progress.clone(),
            config: None,
            file: None,
            bytes_written: 0,
            sequence: 0,
            last_notice: Instant::now(),
        };
        let thread = thread::Builder::new()
            .name("runtime-trace-writer".to_string())
            .spawn(move || writer.run(rx))?;
        Ok(Self {
            tx,
            thread: Mutex::new(Some(thread)),
            queue,
            drops,
            path,
            progress,
        })
    }

    pub fn saturated(&self, importance: TraceImportance) -> bool {
        self.queue.saturated(importance)
    }

    pub fn note_drop(&self, importance: TraceImportance, reason: DropReason) {
        self.drops.note(importance, reason);
    }

    pub fn try_write(
        &self,
        bytes: Vec<u8>,
        generation: u64,
        importance: TraceImportance,
    ) -> TraceRecordDisposition {
        let Some(reservation) = self.queue.reserve(bytes.capacity(), importance) else {
            self.note_drop(importance, DropReason::QueuePressure);
            return TraceRecordDisposition::QueueFull;
        };
        let row = QueuedRow {
            bytes,
            generation,
            importance,
            _reservation: reservation,
        };
        match self.tx.try_send(WriterCommand::Write(row)) {
            Ok(()) => TraceRecordDisposition::Enqueued,
            Err(TrySendError::Full(_)) => {
                self.note_drop(importance, DropReason::QueuePressure);
                TraceRecordDisposition::QueueFull
            }
            Err(TrySendError::Disconnected(_)) => {
                self.note_drop(importance, DropReason::Io);
                TraceRecordDisposition::WriterUnavailable
            }
        }
    }

    pub fn configure(
        &self,
        config: OutputConfig,
        reason: &'static str,
    ) -> io::Result<Receiver<io::Result<()>>> {
        let (done, response) = mpsc::channel();
        self.tx
            .try_send(WriterCommand::Configure(config, reason, done))
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(response)
    }

    pub fn sync(&self) -> TraceFlushStatus {
        self.sync_with_evidence(true)
    }

    pub fn sync_quiet(&self) -> TraceFlushStatus {
        self.sync_with_evidence(false)
    }

    fn sync_with_evidence(&self, evidence: bool) -> TraceFlushStatus {
        let (done, response) = mpsc::channel();
        match self.tx.try_send(WriterCommand::Sync(evidence, done)) {
            Ok(()) => match response.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(())) => {
                    if self.path().is_some() {
                        TraceFlushStatus::Flushed
                    } else {
                        TraceFlushStatus::Off
                    }
                }
                Ok(Err(_)) => TraceFlushStatus::IoError,
                Err(RecvTimeoutError::Timeout) => TraceFlushStatus::Timeout,
                Err(RecvTimeoutError::Disconnected) => TraceFlushStatus::WriterUnavailable,
            },
            Err(TrySendError::Full(_)) => TraceFlushStatus::QueueFull,
            Err(TrySendError::Disconnected(_)) => TraceFlushStatus::WriterUnavailable,
        }
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let progress = self
            .progress
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        progress_snapshot(&progress, &self.queue, &self.drops)
    }

    pub fn segments(&self) -> Vec<TraceSegment> {
        self.progress
            .lock()
            .map(|value| value.files.clone())
            .unwrap_or_default()
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.path.lock().ok().and_then(|path| path.clone())
    }

    #[cfg(test)]
    pub fn usage(&self) -> super::runtime_state::QueueUsage {
        self.queue.snapshot()
    }
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(WriterCommand::Shutdown);
        if let Ok(mut thread) = self.thread.lock() {
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }
}

struct TraceWriter {
    root: PathBuf,
    trace_run_id: String,
    queue: Arc<QueueBudget>,
    drops: Arc<DropCounters>,
    path: Arc<Mutex<Option<PathBuf>>>,
    progress: Arc<Mutex<WriterProgress>>,
    config: Option<OutputConfig>,
    file: Option<BufWriter<File>>,
    bytes_written: u64,
    sequence: u64,
    last_notice: Instant,
}

impl TraceWriter {
    fn run(&mut self, rx: Receiver<WriterCommand>) {
        let mut last_flush = Instant::now();
        loop {
            match rx.recv_timeout(FLUSH_INTERVAL) {
                Ok(command) => {
                    if !self.handle(command) {
                        break;
                    }
                    for _ in 0..BATCH_LIMIT {
                        match rx.try_recv() {
                            Ok(command) => {
                                if !self.handle(command) {
                                    return;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.emit_notice(true);
                    let _ = self.flush();
                    break;
                }
            }
            self.emit_notice(false);
            if last_flush.elapsed() >= FLUSH_INTERVAL {
                if let Err(error) = self.flush() {
                    log::warn!("Runtime trace flush failed: {error}");
                }
                last_flush = Instant::now();
            }
        }
    }

    fn handle(&mut self, command: WriterCommand) -> bool {
        match command {
            WriterCommand::Write(row) => {
                if self.config.map(|config| config.generation) != Some(row.generation) {
                    self.drops.note(row.importance, DropReason::Configuration);
                } else if let Err(error) = self.append(&row.bytes) {
                    let reason = if error.kind() == io::ErrorKind::InvalidData {
                        DropReason::Oversized
                    } else {
                        DropReason::Io
                    };
                    self.drops.note(row.importance, reason);
                    if !matches!(reason, DropReason::Oversized) {
                        if let Ok(mut progress) = self.progress.lock() {
                            progress.io_failures += 1;
                        }
                        log::warn!("Runtime trace write failed: {error}");
                    }
                }
            }
            WriterCommand::Configure(config, reason, done) => {
                self.emit_notice(true);
                let result = self.configure(config, reason);
                let _ = done.send(result);
            }
            WriterCommand::Sync(evidence, done) => {
                self.emit_notice(true);
                let _ = done.send(if evidence {
                    self.flush_checkpoint("sync")
                } else {
                    self.flush()
                });
            }
            WriterCommand::Shutdown => {
                self.emit_notice(true);
                let _ = self.flush_checkpoint("shutdown");
                return false;
            }
        }
        true
    }

    fn configure(&mut self, config: OutputConfig, reason: &str) -> io::Result<()> {
        if config.profile == RuntimeTraceProfile::Off {
            self.flush_checkpoint("traceConfigChanged")?;
            self.file = None;
            self.config = Some(config);
            self.bytes_written = 0;
            self.publish_path(None);
            return Ok(());
        }
        self.open(config, reason)
    }

    fn open(&mut self, config: OutputConfig, reason: &str) -> io::Result<()> {
        create_dir_all(&self.root)?;
        let file_id = TRACE_FILE_ID.fetch_add(1, Ordering::Relaxed) + 1;
        let path = self
            .root
            .join(format!("runtime-trace-{}-{}.jsonl", now_ms(), file_id));
        let progress = self
            .progress
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        let header = encode_row(&json!({
            "schemaVersion": TRACE_SCHEMA_VERSION, "traceRunId": self.trace_run_id, "tsMs": now_ms(),
            "traceMode": config.profile.as_str(), "traceProfile": config.profile.as_str(),
            "dimension": "core", "importance": "essential", "category": "state", "domain": "trace",
            "event": "fileOpened", "sessionId": null,
            "payload": { "fileName": path.file_name().and_then(|name| name.to_str()), "fileSequence": progress.file_sequence + 1, "previousSequence": self.sequence,
                "productionContractVersion": 1, "checkpoint": progress.checkpoint, "reason": reason, "writerThread": thread::current().name(),
                "traceProfile": config.profile.as_str(), "dimensions": config.dimensions.names(),
                "budget": { "maxFileBytes": config.budget.max_file_bytes, "maxFiles": config.budget.max_files,
                    "maxPendingRows": QUEUE_MAX_ROWS, "maxPendingBytes": QUEUE_MAX_BYTES, "maxRowBytes": MAX_ROW_BYTES } },
        }))?;
        if config.budget.max_file_bytes > 0
            && self.line_size(&header) > config.budget.max_file_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "trace file budget is smaller than its header",
            ));
        }
        self.flush()?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let mut next = BufWriter::with_capacity(MAX_ROW_BYTES, file);
        let prefix = self.sequence_prefix();
        next.write_all(prefix.as_bytes())?;
        next.write_all(&header[1..])?;
        next.flush()?;
        self.bytes_written = (prefix.len() + header.len() - 1) as u64;
        self.file = Some(next);
        self.config = Some(config);
        if let Ok(mut progress) = self.progress.lock() {
            progress.file_sequence += 1;
            progress.written_rows += 1;
            progress.written_bytes += self.bytes_written;
            progress.flushed_sequence = self.sequence;
            let file_sequence = progress.file_sequence;
            progress.files.push(TraceSegment {
                file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
                file_sequence,
                first_sequence: self.sequence,
                last_sequence: self.sequence,
                written_bytes: self.bytes_written,
                path: path.clone(),
            });
            if config.budget.max_files > 0 && progress.files.len() > config.budget.max_files {
                progress.files.remove(0);
            }
        }
        self.publish_path(Some(path.clone()));
        // 测试共享运行目录保留并发recorder；裁剪算法以独立目录单测验证。
        #[cfg(not(test))]
        prune_files(&self.root, config.budget.max_files, &path);
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let config = self
            .config
            .ok_or_else(|| io::Error::other("runtime trace output is unavailable"))?;
        if self.file.is_none() {
            return Err(io::Error::other("runtime trace output is off"));
        }
        let limit = config.budget.max_file_bytes;
        if limit > 0 && self.bytes_written.saturating_add(self.line_size(bytes)) > limit {
            // 超大行保留诊断计数，单文件预算包括fileOpened与正文。
            if self.line_size(bytes) > limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runtime trace row exceeds file budget",
                ));
            }
            self.open(config, "budgetRotate")?;
            if self.bytes_written.saturating_add(self.line_size(bytes)) > limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runtime trace row and header exceed file budget",
                ));
            }
        }
        let prefix = self.sequence_prefix();
        // 先计入整行预算，覆盖写入中途失败后的部分落盘。
        self.bytes_written += (prefix.len() + bytes.len() - 1) as u64;
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("runtime trace output is unavailable"))?;
        file.write_all(prefix.as_bytes())?;
        file.write_all(&bytes[1..])?;
        if let Ok(mut progress) = self.progress.lock() {
            progress.written_rows += 1;
            progress.written_bytes += (prefix.len() + bytes.len() - 1) as u64;
            if let Some(segment) = progress.files.last_mut() {
                segment.last_sequence = self.sequence;
                segment.written_bytes = self.bytes_written;
            }
            if bytes
                .windows(b"playbackSessionSummary".len())
                .any(|part| part == b"playbackSessionSummary")
            {
                if let Ok(row) = serde_json::from_slice::<serde_json::Value>(bytes) {
                    progress.checkpoint = Some(
                        json!({"sequence": self.sequence, "sessionId": row["sessionId"], "payload": row["payload"]}),
                    );
                }
            }
        }
        Ok(())
    }

    fn sequence_prefix(&mut self) -> String {
        self.sequence += 1;
        if let Ok(mut progress) = self.progress.lock() {
            progress.sequence = self.sequence;
        }
        format!("{{\"seq\":{},", self.sequence)
    }

    fn line_size(&self, bytes: &[u8]) -> u64 {
        (format!("{{\"seq\":{},", self.sequence + 1).len() + bytes.len() - 1) as u64
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.file.as_mut().map_or(Ok(()), |file| file.flush());
        if let Ok(mut progress) = self.progress.lock() {
            if result.is_ok() {
                progress.flushed_sequence = self.sequence;
            } else {
                progress.io_failures += 1;
            }
        }
        result
    }

    fn flush_checkpoint(&mut self, reason: &str) -> io::Result<()> {
        if self.file.is_none() {
            return Ok(());
        }
        self.flush()?;
        let config = self.config.unwrap();
        let progress = self
            .progress
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        let row = encode_row(&json!({
            "schemaVersion": TRACE_SCHEMA_VERSION, "traceRunId": self.trace_run_id, "tsMs": now_ms(),
            "traceMode": config.profile.as_str(), "traceProfile": config.profile.as_str(),
            "dimension": "core", "importance": "essential", "category": "state", "domain": "trace",
            "event": "traceFlushObserved", "sessionId": null,
            "payload": { "productionContractVersion": 1, "status": "flushed", "flushKind": "bufWriter", "reason": reason,
                "flushedSequence": self.sequence, "backend": progress_snapshot(&progress, &self.queue, &self.drops) },
        }))?;
        self.append(&row)?;
        self.flush()
    }

    fn publish_path(&self, next: Option<PathBuf>) {
        if let Ok(mut path) = self.path.lock() {
            *path = next;
        }
    }

    fn emit_notice(&mut self, force: bool) {
        if self.file.is_none() || !force && self.last_notice.elapsed() < NOTICE_INTERVAL {
            return;
        }
        let dropped = self.drops.take();
        if dropped.total() == 0 {
            return;
        }
        let usage = self.queue.snapshot();
        let config = self.config.unwrap();
        let notice = encode_row(&json!({
            "schemaVersion": TRACE_SCHEMA_VERSION, "traceRunId": self.trace_run_id, "tsMs": now_ms(),
            "traceMode": config.profile.as_str(), "traceProfile": config.profile.as_str(),
            "dimension": "core", "importance": "essential", "category": "state", "domain": "trace",
            "event": "traceBudgetNotice", "sessionId": null,
            "payload": { "reason": "writerQueuePressure", "debugDropped": dropped.debug, "rawDropped": dropped.raw,
                "keyDropped": dropped.key, "essentialDropped": dropped.essential,
                "oversizedDropped": dropped.oversized, "configurationDropped": dropped.configuration, "ioDropped": dropped.io,
                "pendingRows": usage.rows, "pendingBytes": usage.bytes,
                "peakPendingRows": usage.peak_rows, "peakPendingBytes": usage.peak_bytes,
                "maxPendingRows": QUEUE_MAX_ROWS, "maxPendingBytes": QUEUE_MAX_BYTES, "maxRowBytes": MAX_ROW_BYTES,
                "backend": progress_snapshot(&self.progress.lock().map(|value| value.clone()).unwrap_or_default(), &self.queue, &self.drops) },
        }));
        // notice由writer直接提交，满队列时也能保留丢弃证据。
        if notice.and_then(|bytes| self.append(&bytes)).is_err() {
            self.drops.restore(dropped);
        } else {
            self.last_notice = Instant::now();
        }
    }
}

fn progress_snapshot(
    progress: &WriterProgress,
    queue: &QueueBudget,
    drops: &DropCounters,
) -> serde_json::Value {
    let usage = queue.snapshot();
    let dropped = drops.cumulative();
    json!({ "writtenRows": progress.written_rows, "writtenBytes": progress.written_bytes,
        "sequence": progress.sequence, "flushedSequence": progress.flushed_sequence, "ioFailures": progress.io_failures,
        "pendingRows": usage.rows, "pendingBytes": usage.bytes, "peakPendingRows": usage.peak_rows, "peakPendingBytes": usage.peak_bytes,
        "essentialDropped": dropped.essential, "keyDropped": dropped.key, "debugDropped": dropped.debug, "rawDropped": dropped.raw,
        "oversizedDropped": dropped.oversized, "configurationDropped": dropped.configuration, "ioDropped": dropped.io,
        "files": progress.files })
}

fn prune_files(root: &Path, max_files: usize, active_path: &Path) {
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
            if path == active_path
                || !name.starts_with("runtime-trace-")
                || !name.ends_with(".jsonl")
            {
                return None;
            }
            Some((path, entry.metadata().ok()?.modified().ok()?))
        })
        .collect();
    traces.sort_by_key(|(_, modified)| *modified);
    let remove_count = traces.len().saturating_sub(max_files.saturating_sub(1));
    for (path, _) in traces.into_iter().take(remove_count) {
        let _ = remove_file(path);
    }
}

#[cfg(test)]
#[path = "writer.test.rs"]
mod tests;
