use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use super::diagnostics_state::{
    BufferedRow, DiagnosticCapture, DiagnosticIndex, DiagnosticIssue, DiagnosticState, MAX_RECORDS,
    MAX_RECORD_BYTES,
};
use super::policy::{
    default_trace_metadata, RuntimeTraceProfile, TraceDimensionSet, TraceFileBudget, TraceMetadata,
};
use super::runtime_state::{encode_row_with_limit, DropReason, MAX_ROW_BYTES};
use super::trace_contract;
use super::types::{TraceFlushStatus, TraceRecordDisposition};
use super::writer::{OutputConfig, WriterHandle};

pub type RuntimeTraceRecorderRef = Arc<RuntimeTraceRecorder>;
pub(super) const TRACE_SCHEMA_VERSION: u32 = 3;
static TRACE_RECORDER_ID: AtomicU64 = AtomicU64::new(1);

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

#[derive(Clone)]
struct TraceInner {
    trace_mode: String,
    profile: RuntimeTraceProfile,
    dimensions: TraceDimensionSet,
    generation: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceRow<'a> {
    schema_version: u32,
    trace_run_id: &'a str,
    ts_ms: u128,
    trace_mode: &'a str,
    trace_profile: &'a str,
    dimension: &'a str,
    importance: &'a str,
    category: &'a str,
    domain: &'a str,
    event: &'a str,
    session_id: Option<&'a str>,
    payload: Value,
}

/// 发布侧保留过滤与有界编码，文件生命周期由单一writer持有。
pub struct RuntimeTraceRecorder {
    trace_run_id: String,
    inner: Mutex<TraceInner>,
    configuration: Mutex<()>,
    writer: WriterHandle,
    diagnostics: Mutex<DiagnosticState>,
    export_guard: Mutex<()>,
    root: PathBuf,
    // 放在 writer 后，先完成写线程退出与句柄释放，再清理本测试目录。
    #[cfg(test)]
    test_root: tempfile::TempDir,
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
        let profile = RuntimeTraceProfile::from_stored_mode(trace_mode);
        let dimensions = TraceDimensionSet::effective_for_profile(profile, dimensions_expression);
        let budget = profile.budget();
        let trace_run_id = format!(
            "{}-{}-{}",
            now_ms(),
            std::process::id(),
            TRACE_RECORDER_ID.fetch_add(1, Ordering::Relaxed)
        );
        #[cfg(test)]
        let test_root = tempfile::Builder::new()
            .prefix("xbxrc-trace-test-")
            .tempdir()?;
        #[cfg(test)]
        let root = test_root.path().join("runtime-logs");
        #[cfg(not(test))]
        let root = trace_root_dir();
        let index = std::fs::read(root.join("diagnostics-index.json"))
            .ok()
            .filter(|bytes| bytes.len() <= 65536)
            .and_then(|bytes| serde_json::from_slice::<DiagnosticIndex>(&bytes).ok())
            .unwrap_or_default();
        let writer = WriterHandle::new(root.clone(), trace_run_id.clone())?;
        let recorder = Self {
            trace_run_id,
            inner: Mutex::new(TraceInner {
                trace_mode: profile.as_str().to_string(),
                profile,
                dimensions,
                generation: 0,
            }),
            configuration: Mutex::new(()),
            writer,
            diagnostics: Mutex::new(DiagnosticState::new(index)),
            export_guard: Mutex::new(()),
            root,
            #[cfg(test)]
            test_root,
        };
        recorder.open_trace_output(profile, dimensions, budget, "initial")?;
        Ok(recorder)
    }

    fn open_trace_output(
        &self,
        profile: RuntimeTraceProfile,
        dimensions: TraceDimensionSet,
        budget: TraceFileBudget,
        reason: &'static str,
    ) -> std::io::Result<()> {
        let _configuration = self
            .configuration
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let (previous, response) = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            if reason == "traceConfigChanged"
                && inner.profile == profile
                && inner.dimensions == dimensions
            {
                return Ok(());
            }
            let previous = inner.clone();
            let generation = inner.generation + 1;
            let response = self.writer.configure(
                OutputConfig {
                    profile,
                    dimensions,
                    budget,
                    generation,
                },
                reason,
            )?;
            *inner = TraceInner {
                trace_mode: profile.as_str().to_string(),
                profile,
                dimensions,
                generation,
            };
            (previous, response)
        };
        // 启动/显式配置等待writer回执，发布锁在等待IO前已释放。
        let result = response
            .recv()
            .map_err(|error| std::io::Error::other(error.to_string()))
            .and_then(|result| result);
        if result.is_err() {
            if let Ok(mut inner) = self.inner.lock() {
                *inner = previous;
            }
        }
        result
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
        self.open_trace_output(profile, dimensions, budget, "traceConfigChanged")
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.writer.sync_quiet();
        self.writer.path()
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
        self.writer.path().is_some()
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
        let _ = self
            .try_record_value_with_metadata(category, domain, event, session_id, payload, metadata);
    }

    pub fn producer_policy(&self) -> Value {
        let mut policy = trace_contract::producer_policy(self.trace_profile());
        policy["generation"] = json!(self.inner.lock().map(|inner| inner.generation).unwrap_or(0));
        policy
    }

    pub fn status(&self) -> Value {
        let at = now_ms();
        let profile = self.trace_profile();
        let available = self.disk_enabled();
        let mut diagnostics = self
            .diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if diagnostics.expire(at) {
            self.persist_index(&diagnostics);
        }
        let file_info = |id: &str| {
            std::fs::metadata(
                self.root
                    .join("diagnostic-records")
                    .join(format!("diagnostic-record-{id}.jsonl")),
            )
            .ok()
        };
        let mut records=diagnostics.index.issues.iter().map(|issue| {
            let info=file_info(&issue.id);
            json!({"id":issue.id,"kind":"issue","traceRunId":issue.trace_run_id,"startedAtMs":issue.from_ms,"endedAtMs":issue.to_ms,
                "state":if issue.trace_run_id==self.trace_run_id && at<issue.to_ms {"active"} else {"completed"},"count":issue.count,"source":issue.source,"operation":issue.operation,"code":issue.code,
                "available":info.is_some(),"bytes":info.map(|info|info.len()).unwrap_or(0)})
        }).chain(diagnostics.index.captures.iter().map(|capture| {
            let info=file_info(&capture.id);
            json!({"id":capture.id,"kind":"capture","traceRunId":capture.trace_run_id,"startedAtMs":capture.started_at_ms,"endedAtMs":capture.ended_at_ms.unwrap_or(capture.expires_at_ms),
                "state":capture.state,"count":1,"source":"manual","available":info.is_some(),"bytes":info.map(|info|info.len()).unwrap_or(0)})
        })).collect::<Vec<_>>();
        records
            .sort_by_key(|record| std::cmp::Reverse(record["startedAtMs"].as_u64().unwrap_or(0)));
        json!({"traceRunId": self.trace_run_id, "backend": self.writer.snapshot(),
            "diagnostics": { "effectiveMode": if profile == RuntimeTraceProfile::Dev { "development" } else { "production" },
                "lightweight": diagnostics.lightweight, "recordingAvailable": available, "capture": diagnostics.index.capture,
                "latestIssue": diagnostics.index.issues.front(), "bufferedRows": diagnostics.buffer.len(),
                "bufferedBytes": diagnostics.buffered_bytes, "bufferedTotal": diagnostics.buffered_total,
                "contextDropped": diagnostics.context_dropped, "replayDropped": diagnostics.replay_dropped,
                "records":records,"recordRetention":{"maxRecords":MAX_RECORDS,"maxBytes":MAX_RECORD_BYTES},
                "healthPersistenceMs": if !diagnostics.detailed() && profile == RuntimeTraceProfile::Production { if diagnostics.lightweight {30000} else {5000} } else { 1000 } }})
    }

    fn persist_index(&self, state: &DiagnosticState) {
        if let Ok(bytes) = serde_json::to_vec(&state.index) {
            if bytes.len() <= 65536 {
                self.writer.save_diagnostic_index(bytes);
            }
        }
    }

    fn replay_context(&self, rows: Vec<BufferedRow>) {
        let generation = self.inner.lock().map(|inner| inner.generation).unwrap_or(0);
        let mut lost = 0;
        for row in rows {
            if row.generation != generation
                || self.writer.try_write_context(
                    row.bytes,
                    generation,
                    row.importance,
                    row.in_trace,
                ) != TraceRecordDisposition::Enqueued
            {
                lost += 1;
            }
        }
        if let Ok(mut state) = self.diagnostics.lock() {
            state.replay_dropped += lost;
        }
    }

    pub fn apply_lightweight(&self, enabled: bool) {
        let rows = {
            let mut state = self
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.lightweight = enabled;
            if enabled {
                Vec::new()
            } else {
                state.take_context(now_ms())
            }
        };
        self.replay_context(rows);
    }

    pub fn start_capture(&self) -> std::io::Result<DiagnosticCapture> {
        if !self.disk_enabled() {
            return Err(std::io::Error::other("diagnostic recording is unavailable"));
        }
        let (capture, rows) = {
            let mut state = self
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let capture = state.start(&self.trace_run_id, now_ms());
            let rows = state.take_context(now_ms());
            self.persist_index(&state);
            (capture, rows)
        };
        self.writer
            .diagnostic_window(super::diagnostic_records::RecordWindow {
                id: capture.id.clone(),
                from: capture.started_at_ms.saturating_sub(10000),
                to: capture.expires_at_ms,
                manual: true,
            });
        self.replay_context(rows);
        self.record_event("diagnostics", "diagnosticCaptureStarted", None, &capture);
        Ok(capture)
    }

    pub fn stop_capture(&self, expected_id: &str) -> std::io::Result<Option<DiagnosticCapture>> {
        let capture = {
            let mut state = self
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state
                .index
                .capture
                .as_ref()
                .is_none_or(|capture| capture.id != expected_id)
            {
                return Err(std::io::Error::other("diagnostic capture has changed"));
            }
            let capture = state.stop(now_ms());
            self.persist_index(&state);
            capture
        };
        self.record_event("diagnostics", "diagnosticCaptureStopped", None, &capture);
        self.writer.close_diagnostic_window(expected_id);
        Ok(capture)
    }

    pub fn record_request_failure(
        &self,
        operation: &str,
        code: &str,
        started_at_ms: u128,
        duration_ms: u128,
        detail: Value,
    ) -> String {
        let issue = self.note_issue("request", operation, code);
        self.record_event("diagnostics", "diagnosticRequestFailed", None,
            json!({"issueId": issue.id, "operation": operation, "code": code, "startedAtMs": started_at_ms,
                "durationMs": duration_ms, "detail": detail}));
        issue.id
    }

    fn note_issue(&self, source: &str, operation: &str, code: &str) -> DiagnosticIssue {
        let (issue, rows) = {
            let mut state = self
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let issue = state.issue(&self.trace_run_id, source, operation, code, now_ms());
            let rows = state.take_context(now_ms());
            self.persist_index(&state);
            (issue, rows)
        };
        if self.disk_enabled() {
            self.writer
                .diagnostic_window(super::diagnostic_records::RecordWindow {
                    id: issue.id.clone(),
                    from: issue.from_ms,
                    to: issue.to_ms,
                    manual: false,
                });
        }
        self.replay_context(rows);
        issue
    }

    pub(super) fn diagnostic_export_context(
        &self,
        issue_id: Option<&str>,
        capture_id: Option<&str>,
    ) -> std::io::Result<(String, u128, Option<u128>, Value)> {
        let mut state = self
            .diagnostics
            .lock()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        if issue_id.is_some() && capture_id.is_some() {
            return Err(std::io::Error::other("choose one diagnostic reference"));
        }
        if state.expire(now_ms()) {
            self.persist_index(&state);
        }
        if capture_id.is_some_and(|id| state.index.captures.iter().all(|capture| capture.id != id))
        {
            return Err(std::io::Error::other(
                "diagnostic capture is no longer available",
            ));
        }
        let capture = capture_id
            .and_then(|id| state.index.captures.iter().find(|capture| capture.id == id))
            .or_else(|| {
                if issue_id.is_none() {
                    state
                        .index
                        .capture
                        .as_ref()
                        .filter(|capture| capture.state == "active")
                } else {
                    None
                }
            });
        if let Some(capture) = capture {
            return Ok((
                capture.trace_run_id.clone(),
                capture.started_at_ms.saturating_sub(10_000),
                Some(
                    capture
                        .ended_at_ms
                        .unwrap_or(now_ms().min(capture.expires_at_ms)),
                ),
                json!({"capture":capture}),
            ));
        }
        let issue = match issue_id {
            Some(id) => Some(
                state
                    .index
                    .issues
                    .iter()
                    .find(|issue| issue.id == id)
                    .ok_or_else(|| {
                        std::io::Error::other("diagnostic issue is no longer available")
                    })?,
            ),
            None => state.index.issues.front(),
        };
        if let Some(issue) = issue {
            return Ok((
                issue.trace_run_id.clone(),
                issue.from_ms,
                Some(issue.to_ms.min(now_ms())),
                json!({"issue":issue}),
            ));
        }
        if capture_id.is_some() {
            return Err(std::io::Error::other(
                "diagnostic capture is no longer available",
            ));
        }
        Ok((self.trace_run_id.clone(), 0, None, json!({})))
    }

    pub fn export_bundle(&self) -> std::io::Result<Value> {
        let status = self.writer.sync();
        let cutoff = self.writer.snapshot()["flushedSequence"]
            .as_u64()
            .unwrap_or(0);
        super::export::export_bundle(&self.trace_run_id, self.writer.segments(), cutoff, status)
    }

    pub fn flush(&self) -> TraceFlushStatus {
        self.writer.sync()
    }

    pub fn save_diagnostic_bundle(
        &self,
        issue_id: Option<&str>,
        capture_id: Option<&str>,
        configuration: Value,
    ) -> std::io::Result<Value> {
        let _guard = self
            .export_guard
            .try_lock()
            .map_err(|_| std::io::Error::other("diagnostic export is already running"))?;
        let context = self.diagnostic_export_context(issue_id, capture_id)?;
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(&format!("xbxrc-diagnostics-{}.zip", now_ms()))
            .add_filter("Diagnostics", &["zip"])
            .save_file()
        else {
            return Ok(json!({"saved":false,"canceled":true}));
        };
        let rows = self
            .diagnostics
            .lock()
            .map_err(|error| std::io::Error::other(error.to_string()))?
            .take_context(now_ms());
        self.replay_context(rows);
        let status = self.writer.sync();
        let cutoff = self.writer.snapshot()["flushedSequence"]
            .as_u64()
            .unwrap_or(0);
        super::diagnostic_export::save(
            &path,
            &self.root,
            self.writer.segments(),
            &context,
            cutoff,
            status,
            self.status(),
            configuration,
            self.trace_profile(),
        )?;
        Ok(json!({"saved":true,"canceled":false,"path":path.display().to_string()}))
    }

    pub fn try_record_value_with_metadata(
        &self,
        category: RuntimeTraceCategory,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: Value,
        metadata: TraceMetadata,
    ) -> TraceRecordDisposition {
        let automatic_issue = (event == "browserPlaybackAnomalyObserved"
            && matches!(payload["phase"].as_str(), Some("start" | "upgrade")))
            || (event == "inputUiInteractionObserved"
                && matches!(payload["outcome"].as_str(), Some("failed" | "hardFault")));
        if automatic_issue {
            self.note_issue(
                if event.starts_with("input") {
                    "input"
                } else {
                    "media"
                },
                event,
                "runtime-anomaly",
            );
        }
        let now = now_ms();
        let Ok(inner) = self.inner.lock() else {
            return TraceRecordDisposition::WriterUnavailable;
        };
        let metadata = trace_contract::metadata(event, &payload).unwrap_or(metadata);
        if inner.profile == RuntimeTraceProfile::Off
            || !super::policy::should_record_trace_event(
                inner.profile,
                inner.dimensions,
                metadata,
                domain,
                event,
                now as f64,
            )
        {
            return TraceRecordDisposition::Filtered;
        }
        if self.writer.saturated(metadata.importance) {
            self.writer
                .note_drop(metadata.importance, DropReason::QueuePressure);
            return TraceRecordDisposition::QueueFull;
        }
        let contract = trace_contract::contract(event)
            .filter(|_| inner.profile == RuntimeTraceProfile::Production);
        let category = contract
            .and_then(|contract| RuntimeTraceCategory::parse(contract.category))
            .unwrap_or(category);
        let payload = if contract.is_some() {
            trace_contract::project_payload(event, &payload)
        } else {
            payload
        };
        let line = TraceRow {
            schema_version: TRACE_SCHEMA_VERSION,
            trace_run_id: &self.trace_run_id,
            ts_ms: now,
            trace_mode: &inner.trace_mode,
            trace_profile: inner.profile.as_str(),
            dimension: metadata.dimension.as_str(),
            importance: metadata.importance.as_str(),
            category: category.as_str(),
            domain,
            event,
            session_id,
            payload,
        };
        match encode_row_with_limit(
            &line,
            contract.map_or(MAX_ROW_BYTES, |contract| contract.max_row_bytes),
        ) {
            Ok(bytes) => {
                let mut state = self
                    .diagnostics
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.expire(now) {
                    self.persist_index(&state);
                }
                if inner.profile == RuntimeTraceProfile::Production
                    && !state.detailed()
                    && super::diagnostics_state::periodic(event)
                {
                    let disposition = if state.sparse(event, session_id) {
                        TraceRecordDisposition::Buffered
                    } else {
                        self.writer
                            .try_write(bytes.clone(), inner.generation, metadata.importance)
                    };
                    state.buffer(
                        BufferedRow {
                            in_trace: disposition == TraceRecordDisposition::Enqueued,
                            at_ms: now,
                            bytes,
                            generation: inner.generation,
                            importance: metadata.importance,
                        },
                        now,
                    );
                    return disposition;
                }
                drop(state);
                self.writer
                    .try_write(bytes, inner.generation, metadata.importance)
            }
            Err(_) => {
                self.writer
                    .note_drop(metadata.importance, DropReason::Oversized);
                TraceRecordDisposition::RowOversized
            }
        }
    }
}

/// 运行期 trace 根目录覆盖，由 shell 在启动时指向应用日志目录。
///
/// 历史实现用 `CARGO_MANIFEST_DIR` 推导落盘目录，等于把开发机的源码路径写进了
/// 二进制：CI 打包后路径变成 `D:\a\<repo>\<repo>`，安装到其它机器上不可创建时
/// 会让 `init_services` 直接失败并静默退出。因此这里改为运行期注入。
static TRACE_ROOT_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// 设置 trace 落盘根目录；进程内只生效一次，重复调用会被忽略。
pub fn set_trace_root_dir(root: PathBuf) {
    let _ = TRACE_ROOT_OVERRIDE.set(root);
}

pub fn trace_root_dir() -> PathBuf {
    TRACE_ROOT_OVERRIDE
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("xbxrc").join("runtime-logs"))
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::super::policy::{TraceDimension, TraceImportance};
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
    fn failed_configuration_restores_publisher_profile_dimensions_and_generation() {
        let recorder = RuntimeTraceRecorder::new_with_mode("dev").unwrap();
        let before = recorder.inner.lock().unwrap().clone();
        let first_path = recorder.path().unwrap();
        let profile = RuntimeTraceProfile::Production;
        assert!(recorder
            .open_trace_output(
                profile,
                TraceDimensionSet::default_for_profile(profile),
                TraceFileBudget {
                    max_file_bytes: 1,
                    max_files: 1,
                },
                "traceConfigChanged",
            )
            .is_err());
        assert_eq!(recorder.trace_profile(), before.profile);
        assert_eq!(recorder.trace_dimensions(), before.dimensions);
        assert_eq!(recorder.inner.lock().unwrap().generation, before.generation);
        assert_eq!(recorder.path().unwrap(), first_path);
        recorder.record_event("test", "afterConfigRollback", None, json!({}));
        let entries = read_trace_lines(&recorder);
        let row = entries
            .iter()
            .find(|row| row["event"] == "afterConfigRollback")
            .unwrap();
        assert_eq!(row["traceProfile"], "dev");
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
        let dimensions = recorder.trace_dimensions();
        recorder
            .open_trace_output(
                RuntimeTraceProfile::Dev,
                dimensions,
                TraceFileBudget {
                    max_file_bytes: 4096,
                    max_files: 10,
                },
                "testBudget",
            )
            .unwrap();
        let first_path = recorder.path().expect("first path");
        for _ in 0..2 {
            recorder.record_event(
                "xbxengine",
                "largeTraceRow",
                None,
                json!({ "blob": "x".repeat(2200) }),
            );
        }
        let second_path = recorder.path().expect("second path");
        assert_ne!(first_path, second_path);
        let second_contents = fs::read_to_string(&second_path).expect("second trace contents");
        assert!(second_contents.contains("budgetRotate"));
        assert!(second_contents.contains("largeTraceRow"));
        assert!(fs::metadata(first_path).unwrap().len() <= 4096);
        assert!(fs::metadata(second_path).unwrap().len() <= 4096);
    }

    #[test]
    fn oversized_rows_leave_essential_drop_evidence() {
        let recorder = RuntimeTraceRecorder::new_with_mode("dev").unwrap();
        recorder.record_with_metadata(
            RuntimeTraceCategory::Event,
            "test",
            "oversized",
            None,
            json!({ "blob": "x".repeat(super::super::runtime_state::MAX_ROW_BYTES) }),
            TraceMetadata::new(TraceDimension::Core, TraceImportance::Essential),
        );
        let entries = read_trace_lines(&recorder);
        assert!(!entries.iter().any(|entry| entry["event"] == "oversized"));
        let notice = entries
            .iter()
            .find(|entry| entry["event"] == "traceBudgetNotice")
            .unwrap();
        assert_eq!(notice["payload"]["essentialDropped"], 1);
        assert_eq!(notice["payload"]["oversizedDropped"], 1);
        assert_eq!(recorder.writer.usage().bytes, 0);
    }

    #[test]
    fn off_mode_disables_disk_output() {
        let recorder = RuntimeTraceRecorder::new_with_mode("off").expect("trace recorder");
        recorder.record_event("test", "ignored", None, json!({}));

        assert!(!recorder.disk_enabled());
        assert!(recorder.path().is_none());
    }

    #[test]
    fn independent_recorders_preserve_live_output_and_clean_up_their_own_directory() {
        let first = RuntimeTraceRecorder::new_with_mode("dev").unwrap();
        first.record_event("test", "retained", None, json!({}));
        first.writer.sync();
        let first_path = first.path().unwrap();
        let first_root = first.test_root.path().to_path_buf();
        let second = RuntimeTraceRecorder::new_with_mode("dev").unwrap();
        let second_root = second.test_root.path().to_path_buf();
        assert_ne!(first_root, second_root);
        assert!(fs::read_to_string(&first_path)
            .unwrap()
            .contains("retained"));
        drop(second);
        assert!(!second_root.exists());
        assert!(first_path.exists());
        drop(first);
        assert!(!first_root.exists());
    }
    #[test]
    fn regular_and_lightweight_buffer_health_and_pin_error_windows() {
        let recorder = RuntimeTraceRecorder::new_with_mode("production").unwrap();
        assert_eq!(
            recorder.status()["diagnostics"]["healthPersistenceMs"],
            5000
        );
        recorder.apply_lightweight(true);
        assert_eq!(
            recorder.status()["diagnostics"]["healthPersistenceMs"],
            30000
        );
        for sequence in 0..30 {
            recorder.record_event(
                "streaming-runtime-host",
                "browserPlaybackHealthObserved",
                Some("session"),
                json!({"healthSequence":sequence,"frameSupply":{"receivedFps":60}}),
            );
        }
        assert_eq!(recorder.flush(), TraceFlushStatus::Flushed);
        let status = recorder.status();
        assert_eq!(status["diagnostics"]["bufferedTotal"], 30);
        assert!(status["backend"]["writtenRows"].as_u64().unwrap() < 10);
        let id = recorder.record_request_failure(
            "data.getHosts",
            "DATA_ERROR",
            now_ms(),
            10,
            json!({"statusCode":500}),
        );
        recorder.flush();
        assert_eq!(recorder.status()["diagnostics"]["records"][0]["id"], id);
        assert_eq!(
            recorder.status()["diagnostics"]["records"][0]["available"],
            true
        );
        let saved = recorder
            .root
            .join("diagnostic-records")
            .join(format!("diagnostic-record-{id}.jsonl"));
        let contents = fs::read_to_string(&saved).unwrap();
        let records = contents
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(records
            .windows(2)
            .all(|rows| rows[0]["seq"].as_u64() < rows[1]["seq"].as_u64()));
        let health = records
            .iter()
            .filter(|row| row["event"] == "browserPlaybackHealthObserved")
            .collect::<Vec<_>>();
        assert_eq!(health.len(), 30);
        assert!(health
            .iter()
            .any(|row| row["payload"]["healthSequence"] == 0));
        assert!(contents.contains("healthSequence"));
        assert!(contents.contains("diagnosticRequestFailed"));
        let index = serde_json::from_slice::<DiagnosticIndex>(
            &fs::read(recorder.root.join("diagnostics-index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(index.issues.front().unwrap().id, id);
    }
    #[test]
    fn manual_windows_survive_rotation_and_previous_captures_remain_selectable() {
        let recorder = RuntimeTraceRecorder::new_with_mode("production").unwrap();
        let first = recorder.start_capture().unwrap();
        recorder.record_event("test", "operationCompleted", None, json!({"durationMs":10}));
        recorder.stop_capture(&first.id).unwrap();
        let second = recorder.start_capture().unwrap();
        recorder.stop_capture(&second.id).unwrap();
        recorder.flush();
        assert_eq!(
            recorder.status()["diagnostics"]["records"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            recorder
                .diagnostic_export_context(None, Some(&first.id))
                .unwrap()
                .3["capture"]["id"],
            first.id
        );
        assert!(recorder
            .diagnostic_export_context(None, Some("stale"))
            .is_err());
        for segment in recorder.writer.segments() {
            let _ = fs::remove_file(segment.path);
        }
        assert_eq!(
            recorder.status()["diagnostics"]["records"][0]["available"],
            true
        );
        let context = recorder
            .diagnostic_export_context(None, Some(&first.id))
            .unwrap();
        let output = recorder.root.join("saved.zip");
        super::super::diagnostic_export::save(
            &output,
            &recorder.root,
            Vec::new(),
            &context,
            u64::MAX,
            TraceFlushStatus::Flushed,
            recorder.status(),
            json!({}),
            RuntimeTraceProfile::Production,
        )
        .unwrap();
        assert!(output.exists());
    }
}
