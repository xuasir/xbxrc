use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

pub type RuntimeTraceRecorderRef = Arc<RuntimeTraceRecorder>;

/// 开发期协助日志：独立写入项目目录，便于后续整体清理。
pub struct RuntimeTraceRecorder {
    file: Mutex<File>,
    path: PathBuf,
}

impl RuntimeTraceRecorder {
    pub fn new() -> std::io::Result<Self> {
        let root = project_root().join("runtime-logs");
        create_dir_all(&root)?;
        let path = root.join(format!("runtime-trace-{}.jsonl", now_ms()));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let recorder = Self {
            file: Mutex::new(file),
            path,
        };
        recorder.record(
            "trace",
            "fileOpened",
            None,
            json!({
                "path": recorder.path,
            }),
        );
        Ok(recorder)
    }

    pub fn path(&self) -> &Path {
        &self.path
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
        self.record_value(domain, event, session_id, payload);
    }

    pub fn record_value(
        &self,
        domain: &str,
        event: &str,
        session_id: Option<&str>,
        payload: Value,
    ) {
        let line = json!({
            "tsMs": now_ms(),
            "domain": domain,
            "event": event,
            "sessionId": session_id,
            "payload": payload,
        });
        let Ok(mut file) = self.file.lock() else {
            return;
        };
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
