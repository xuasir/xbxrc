#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use super::policy::{
    default_trace_metadata, RuntimeTraceProfile, TraceDimensionSet, TraceFileBudget, TraceMetadata,
};
use super::runtime_state::{encode_row, DropReason};
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
        let writer = WriterHandle::new(trace_root_dir(), trace_run_id.clone())?;
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
        self.writer.sync();
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
        let now = now_ms();
        let Ok(inner) = self.inner.lock() else {
            return;
        };
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
            return;
        }
        if self.writer.saturated(metadata.importance) {
            self.writer
                .note_drop(metadata.importance, DropReason::QueuePressure);
            return;
        }
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
        match encode_row(&line) {
            Ok(bytes) => self
                .writer
                .try_write(bytes, inner.generation, metadata.importance),
            Err(_) => self
                .writer
                .note_drop(metadata.importance, DropReason::Oversized),
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

#[cfg(test)]
fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

pub fn trace_root_dir() -> PathBuf {
    #[cfg(test)]
    {
        current_test_trace_run_dir()
    }
    #[cfg(not(test))]
    {
        TRACE_ROOT_OVERRIDE
            .get()
            .cloned()
            .unwrap_or_else(|| std::env::temp_dir().join("xbxrc").join("runtime-logs"))
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
    let _ = std::fs::create_dir_all(&run_dir);
    run_dir
}

pub(super) fn now_ms() -> u128 {
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
