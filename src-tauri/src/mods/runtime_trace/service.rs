use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

pub type RuntimeTraceRecorderRef = Arc<RuntimeTraceRecorder>;

const TRACE_SCHEMA_VERSION: u32 = 2;

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
}

struct TraceInner {
    trace_mode: String,
    file: Option<File>,
    path: Option<PathBuf>,
}

/// 开发期协助日志：独立写入项目目录，便于后续整体清理。
pub struct RuntimeTraceRecorder {
    inner: Mutex<TraceInner>,
    sequence: AtomicU64,
}

impl RuntimeTraceRecorder {
    /// `trace_mode` 与设置项 `runtime_trace_mode` 一致；`off` 时不创建文件、不写盘。
    pub fn new_with_mode(trace_mode: &str) -> std::io::Result<Self> {
        let trace_mode = trace_mode.to_string();
        let inner = TraceInner {
            trace_mode: trace_mode.clone(),
            file: None,
            path: None,
        };
        let recorder = Self {
            inner: Mutex::new(inner),
            sequence: AtomicU64::new(0),
        };
        if trace_mode != "off" {
            recorder.apply_trace_mode_open_file(&trace_mode)?;
        }
        Ok(recorder)
    }

    fn apply_trace_mode_open_file(&self, trace_mode: &str) -> std::io::Result<()> {
        let root = project_root().join("runtime-logs");
        create_dir_all(&root)?;
        let path = root.join(format!("runtime-trace-{}.jsonl", now_ms()));
        let path_open_payload = path.display().to_string();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            inner.file = Some(file);
            inner.path = Some(path);
            inner.trace_mode = trace_mode.to_string();
        }
        self.record_state(
            "trace",
            "fileOpened",
            None,
            json!({
                "path": path_open_payload,
            }),
        );
        Ok(())
    }

    /// 设置页修改 `runtime_trace_mode` 后立即应用：开关盘写入、更新当前模式字段。
    pub fn apply_trace_mode(&self, mode: &str) -> std::io::Result<()> {
        let mode = mode.to_string();
        let path_open_payload = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            if inner.trace_mode == mode {
                return Ok(());
            }
            if let Some(mut file) = inner.file.take() {
                let _ = file.flush();
                inner.path = None;
            }
            inner.trace_mode = mode.clone();
            if mode == "off" {
                return Ok(());
            }
            let root = project_root().join("runtime-logs");
            create_dir_all(&root)?;
            let path = root.join(format!("runtime-trace-{}.jsonl", now_ms()));
            let path_open_payload = path.display().to_string();
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            inner.file = Some(file);
            inner.path = Some(path.clone());
            Some(path_open_payload)
        };
        if let Some(path_open_payload) = path_open_payload {
            self.record_state(
                "trace",
                "fileOpened",
                None,
                json!({
                    "path": path_open_payload,
                    "reason": "traceModeChanged",
                }),
            );
        }
        Ok(())
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.inner.lock().ok().and_then(|g| g.path.clone())
    }

    pub fn trace_mode(&self) -> String {
        self.inner
            .lock()
            .map(|g| g.trace_mode.clone())
            .unwrap_or_default()
    }

    pub fn disk_enabled(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .map(|g| g.file.is_some())
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
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let trace_mode = inner.trace_mode.clone();
        let Some(file) = inner.file.as_mut() else {
            return;
        };
        let line = json!({
            "schemaVersion": TRACE_SCHEMA_VERSION,
            "seq": self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            "tsMs": now_ms(),
            "traceMode": trace_mode,
            "category": category.as_str(),
            "domain": domain,
            "event": event,
            "sessionId": session_id,
            "payload": payload,
        });
        if serde_json::to_writer(&mut *file, &line).is_err() {
            return;
        }
        let _ = file.write_all(b"\n");
        let _ = file.flush();
    }
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}
