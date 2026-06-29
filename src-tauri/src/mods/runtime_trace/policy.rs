//! runtime-logs 写入策略：profile 决定保留强度，dimension 决定诊断面。
use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use xbxengine::XbxLogLevel;

static TRACE_EVENT_SAMPLE_TS_MS: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();

pub const PRODUCTION_TRACE_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const PRODUCTION_TRACE_MAX_FILES: usize = 5;
pub const DEV_TRACE_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const DEV_TRACE_MAX_FILES: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceFileBudget {
    pub max_file_bytes: u64,
    pub max_files: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeTraceProfile {
    Off,
    Production,
    Dev,
}

impl RuntimeTraceProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeTraceProfile::Off => "off",
            RuntimeTraceProfile::Production => "production",
            RuntimeTraceProfile::Dev => "dev",
        }
    }

    pub fn from_stored_mode(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "0" => RuntimeTraceProfile::Off,
            "dev" | "debug" | "verbose" | "trace" => RuntimeTraceProfile::Dev,
            "production" | "prod" | "minimal" | "standard" => RuntimeTraceProfile::Production,
            _ => RuntimeTraceProfile::Production,
        }
    }

    pub fn effective_from_stored_mode(raw: &str) -> Self {
        let profile = Self::from_stored_mode(raw);
        if cfg!(debug_assertions) {
            profile
        } else {
            match profile {
                RuntimeTraceProfile::Dev => RuntimeTraceProfile::Production,
                other => other,
            }
        }
    }

    pub fn budget(self) -> TraceFileBudget {
        match self {
            RuntimeTraceProfile::Off => TraceFileBudget {
                max_file_bytes: 0,
                max_files: 0,
            },
            RuntimeTraceProfile::Production => TraceFileBudget {
                max_file_bytes: PRODUCTION_TRACE_MAX_FILE_BYTES,
                max_files: PRODUCTION_TRACE_MAX_FILES,
            },
            RuntimeTraceProfile::Dev => TraceFileBudget {
                max_file_bytes: DEV_TRACE_MAX_FILE_BYTES,
                max_files: DEV_TRACE_MAX_FILES,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TraceDimension {
    Core,
    Lifecycle,
    Network,
    Recovery,
    MediaSupply,
    Presentation,
    Input,
    NativeVideo,
    Frontend,
    EngineLog,
}

impl TraceDimension {
    pub const ALL: [TraceDimension; 10] = [
        TraceDimension::Core,
        TraceDimension::Lifecycle,
        TraceDimension::Network,
        TraceDimension::Recovery,
        TraceDimension::MediaSupply,
        TraceDimension::Presentation,
        TraceDimension::Input,
        TraceDimension::NativeVideo,
        TraceDimension::Frontend,
        TraceDimension::EngineLog,
    ];

    fn bit(self) -> u16 {
        1 << (self as u16)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TraceDimension::Core => "core",
            TraceDimension::Lifecycle => "lifecycle",
            TraceDimension::Network => "network",
            TraceDimension::Recovery => "recovery",
            TraceDimension::MediaSupply => "media_supply",
            TraceDimension::Presentation => "presentation",
            TraceDimension::Input => "input",
            TraceDimension::NativeVideo => "native_video",
            TraceDimension::Frontend => "frontend",
            TraceDimension::EngineLog => "engine_log",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "core" => Some(TraceDimension::Core),
            "lifecycle" | "life_cycle" => Some(TraceDimension::Lifecycle),
            "network" | "transport" | "rtc" => Some(TraceDimension::Network),
            "recovery" | "recover" => Some(TraceDimension::Recovery),
            "media_supply" | "media" | "supply" => Some(TraceDimension::MediaSupply),
            "presentation" | "present" | "display" | "render" => Some(TraceDimension::Presentation),
            "input" | "gamepad" => Some(TraceDimension::Input),
            "native_video" | "nativevideo" => Some(TraceDimension::NativeVideo),
            "frontend" | "front_end" | "ui" => Some(TraceDimension::Frontend),
            "engine_log" | "engine_logs" | "log" | "logs" => Some(TraceDimension::EngineLog),
            _ => None,
        }
    }
}

impl fmt::Display for TraceDimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceDimensionSet {
    mask: u16,
}

impl TraceDimensionSet {
    pub fn empty() -> Self {
        Self { mask: 0 }
    }

    pub fn all() -> Self {
        let mut set = Self::empty();
        for dimension in TraceDimension::ALL {
            set.insert(dimension);
        }
        set
    }

    pub fn default_for_profile(profile: RuntimeTraceProfile) -> Self {
        match profile {
            RuntimeTraceProfile::Off => Self::empty(),
            RuntimeTraceProfile::Production => Self::from_dimensions(&[
                TraceDimension::Core,
                TraceDimension::Lifecycle,
                TraceDimension::Network,
                TraceDimension::Recovery,
                TraceDimension::MediaSupply,
                TraceDimension::Presentation,
                TraceDimension::Frontend,
                TraceDimension::NativeVideo,
            ]),
            RuntimeTraceProfile::Dev => {
                let mut set = Self::all();
                set.remove(TraceDimension::EngineLog);
                set
            }
        }
    }

    pub fn effective_for_profile(
        profile: RuntimeTraceProfile,
        raw_expression: Option<&str>,
    ) -> Self {
        if profile != RuntimeTraceProfile::Dev {
            return Self::default_for_profile(profile);
        }
        let Some(raw_expression) = raw_expression
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Self::default_for_profile(profile);
        };

        let tokens: Vec<&str> = raw_expression
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .collect();
        let has_positive = tokens.iter().any(|token| !token.starts_with('-'));
        let mut set = if has_positive {
            Self::empty()
        } else {
            Self::default_for_profile(profile)
        };

        for token in tokens {
            let remove = token.starts_with('-');
            let name = token.trim_start_matches('-');
            if name.eq_ignore_ascii_case("all") {
                if remove {
                    set = Self::empty();
                } else {
                    set = Self::all();
                }
                continue;
            }
            let Some(dimension) = TraceDimension::parse(name) else {
                continue;
            };
            if remove {
                set.remove(dimension);
            } else {
                set.insert(dimension);
            }
        }
        set
    }

    pub fn from_dimensions(dimensions: &[TraceDimension]) -> Self {
        let mut set = Self::empty();
        for dimension in dimensions {
            set.insert(*dimension);
        }
        set
    }

    pub fn insert(&mut self, dimension: TraceDimension) {
        self.mask |= dimension.bit();
    }

    pub fn remove(&mut self, dimension: TraceDimension) {
        self.mask &= !dimension.bit();
    }

    pub fn contains(self, dimension: TraceDimension) -> bool {
        self.mask & dimension.bit() != 0
    }

    pub fn names(self) -> Vec<&'static str> {
        TraceDimension::ALL
            .iter()
            .copied()
            .filter(|dimension| self.contains(*dimension))
            .map(TraceDimension::as_str)
            .collect()
    }

    pub fn expression(self) -> String {
        self.names().join(",")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TraceImportance {
    Essential,
    Key,
    Debug,
    Raw,
}

impl TraceImportance {
    pub fn as_str(self) -> &'static str {
        match self {
            TraceImportance::Essential => "essential",
            TraceImportance::Key => "key",
            TraceImportance::Debug => "debug",
            TraceImportance::Raw => "raw",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "essential" => Some(TraceImportance::Essential),
            "key" => Some(TraceImportance::Key),
            "debug" => Some(TraceImportance::Debug),
            "raw" => Some(TraceImportance::Raw),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceMetadata {
    pub dimension: TraceDimension,
    pub importance: TraceImportance,
}

impl TraceMetadata {
    pub fn new(dimension: TraceDimension, importance: TraceImportance) -> Self {
        Self {
            dimension,
            importance,
        }
    }
}

/// 按 profile 和维度调整引擎日志是否进入 runtime trace sink。
pub fn apply_xbxengine_trace_logging(profile: RuntimeTraceProfile, dimensions: TraceDimensionSet) {
    xbxengine::set_stderr_enabled(false);
    match profile {
        RuntimeTraceProfile::Off => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Warn));
            xbxengine::set_log_sink_min_level(None);
        }
        RuntimeTraceProfile::Production => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Warn));
            xbxengine::set_log_sink_min_level(Some(XbxLogLevel::Error));
        }
        RuntimeTraceProfile::Dev => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Debug));
            let min_level = if dimensions.contains(TraceDimension::EngineLog) {
                Some(XbxLogLevel::Debug)
            } else {
                Some(XbxLogLevel::Error)
            };
            xbxengine::set_log_sink_min_level(min_level);
        }
    }
}

/// `statsSnapshot` / `observabilitySnapshot` 写入间隔：生产克制，dev 保留细粒度。
pub fn stats_snapshot_interval(profile: RuntimeTraceProfile) -> Duration {
    match profile {
        RuntimeTraceProfile::Production => Duration::from_secs(5),
        RuntimeTraceProfile::Dev => Duration::from_secs(1),
        RuntimeTraceProfile::Off => Duration::from_secs(5),
    }
}

/// `record_runtime_trace_observations` 最小调用间隔（与 xbxengine 16ms tick 解耦）。
pub fn trace_observation_tick_interval(profile: RuntimeTraceProfile) -> Duration {
    match profile {
        RuntimeTraceProfile::Production => Duration::from_millis(500),
        RuntimeTraceProfile::Dev => Duration::from_millis(16),
        RuntimeTraceProfile::Off => Duration::from_millis(500),
    }
}

fn trace_event_sample_slot() -> &'static Mutex<HashMap<String, f64>> {
    TRACE_EVENT_SAMPLE_TS_MS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn should_emit_sampled_trace_event(now_ms: f64, key: &str, interval_ms: f64) -> bool {
    let Ok(mut slot) = trace_event_sample_slot().lock() else {
        return false;
    };
    let should_emit = slot
        .get(key)
        .map(|last| now_ms - *last >= interval_ms)
        .unwrap_or(true);
    if should_emit {
        slot.insert(key.to_string(), now_ms);
    }
    should_emit
}

pub fn default_trace_metadata(
    category: &str,
    domain: &str,
    event: &str,
    payload: &Value,
) -> TraceMetadata {
    if category == "log" {
        return trace_log_metadata(domain, payload);
    }

    let dimension = classify_dimension(domain, event);
    let importance = classify_importance(category, domain, event);
    TraceMetadata::new(dimension, importance)
}

fn trace_log_metadata(domain: &str, payload: &Value) -> TraceMetadata {
    let level = payload
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_uppercase();
    if domain == "native_video" && level.is_empty() {
        return TraceMetadata::new(TraceDimension::NativeVideo, TraceImportance::Key);
    }
    let importance = if level == "ERROR" {
        TraceImportance::Essential
    } else if level == "WARN" {
        TraceImportance::Debug
    } else {
        TraceImportance::Raw
    };
    let dimension = if domain == "native_video" {
        TraceDimension::NativeVideo
    } else {
        TraceDimension::EngineLog
    };
    TraceMetadata::new(dimension, importance)
}

fn classify_dimension(domain: &str, event: &str) -> TraceDimension {
    let key = format!("{domain}:{event}").to_ascii_lowercase();
    if domain == "trace" {
        return TraceDimension::Core;
    }
    if domain.contains("native_video") || domain == "native_video" {
        return TraceDimension::NativeVideo;
    }
    if key.contains("gamepad") || key.contains("input") || key.contains("rumble") {
        return TraceDimension::Input;
    }
    if key.contains("frontend")
        || key.contains("runtime-host")
        || key.contains("browser")
        || key.contains("rendertelemetry")
        || key.contains("renderpolicy")
    {
        return TraceDimension::Frontend;
    }
    if key.contains("ice")
        || key.contains("turn")
        || key.contains("twcc")
        || key.contains("candidate")
        || key.contains("transport")
        || key.contains("channel")
        || key.contains("keepalive")
        || key.contains("sdp")
    {
        return TraceDimension::Network;
    }
    if key.contains("recover")
        || key.contains("keyframe")
        || key.contains("nack")
        || key.contains("h264")
        || key.contains("reference")
        || key.contains("insertgate")
        || key.contains("chain")
        || key.contains("idr")
    {
        return TraceDimension::Recovery;
    }
    if key.contains("present")
        || key.contains("render")
        || key.contains("hostmailbox")
        || key.contains("hostframe")
        || key.contains("mailbox")
        || key.contains("display")
    {
        return TraceDimension::Presentation;
    }
    if key.contains("decode")
        || key.contains("pacer")
        || key.contains("media")
        || key.contains("frame")
        || key.contains("supply")
        || key.contains("fps")
    {
        return TraceDimension::MediaSupply;
    }
    if key.contains("runtime")
        || key.contains("session")
        || key.contains("launch")
        || key.contains("stop")
    {
        return TraceDimension::Lifecycle;
    }
    TraceDimension::Core
}

fn classify_importance(category: &str, domain: &str, event: &str) -> TraceImportance {
    if domain == "trace" || event == "runtimeBuildInfo" {
        return TraceImportance::Essential;
    }
    if event.ends_with("Raw") || event.contains("Raw") {
        return TraceImportance::Raw;
    }
    if event == "runtimeEventRaw" || event == "channelMessageCatalog" {
        return TraceImportance::Debug;
    }
    match category {
        "state" | "decision" => TraceImportance::Key,
        "snapshot" => TraceImportance::Key,
        "event" => TraceImportance::Key,
        _ => TraceImportance::Debug,
    }
}

/// 高频 UI / 原始事件在 production 下降采样；关键结构化事件仍全量保留。
pub fn should_record_trace_event(
    profile: RuntimeTraceProfile,
    dimensions: TraceDimensionSet,
    metadata: TraceMetadata,
    domain: &str,
    event: &str,
    now_ms: f64,
) -> bool {
    if profile == RuntimeTraceProfile::Off {
        return false;
    }
    if metadata.importance == TraceImportance::Essential {
        return true;
    }
    if !dimensions.contains(metadata.dimension) {
        return false;
    }
    match profile {
        RuntimeTraceProfile::Production => {
            if metadata.importance > TraceImportance::Key {
                return false;
            }
            match (domain, event) {
                ("gamepad-shell", "runtimeSnapshotTransitionObserved") => {
                    should_emit_sampled_trace_event(
                        now_ms,
                        "gamepad-shell:runtimeSnapshot",
                        2_000.0,
                    )
                }
                (
                    "streaming-runtime-host",
                    "gamepadUiRuntimeSnapshotApplied" | "gamepadUiRuntimeSnapshotRefreshed",
                ) => should_emit_sampled_trace_event(
                    now_ms,
                    &format!("streaming-runtime-host:{event}"),
                    2_000.0,
                ),
                ("xbxengine", "runtimeEventRaw") => {
                    should_emit_sampled_trace_event(now_ms, "xbxengine:runtimeEventRaw", 1_000.0)
                }
                ("xbxengine", "channelMessageCatalog") => should_emit_sampled_trace_event(
                    now_ms,
                    "xbxengine:channelMessageCatalog",
                    2_000.0,
                ),
                ("native_video", "layout_updated") => {
                    should_emit_sampled_trace_event(now_ms, "native_video:layout_updated", 2_000.0)
                }
                _ => true,
            }
        }
        RuntimeTraceProfile::Dev => true,
        RuntimeTraceProfile::Off => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_modes_normalize_to_profiles() {
        assert_eq!(
            RuntimeTraceProfile::from_stored_mode("minimal"),
            RuntimeTraceProfile::Production
        );
        assert_eq!(
            RuntimeTraceProfile::from_stored_mode("standard"),
            RuntimeTraceProfile::Production
        );
        assert_eq!(
            RuntimeTraceProfile::from_stored_mode("verbose"),
            RuntimeTraceProfile::Dev
        );
        assert_eq!(
            RuntimeTraceProfile::from_stored_mode("trace"),
            RuntimeTraceProfile::Dev
        );
    }

    #[test]
    fn production_uses_fixed_dimensions() {
        let dimensions =
            TraceDimensionSet::effective_for_profile(RuntimeTraceProfile::Production, Some("all"));
        assert!(dimensions.contains(TraceDimension::Recovery));
        assert!(!dimensions.contains(TraceDimension::Input));
        assert!(!dimensions.contains(TraceDimension::EngineLog));
    }

    #[test]
    fn dev_dimension_expression_supports_add_and_remove() {
        let dimensions = TraceDimensionSet::effective_for_profile(
            RuntimeTraceProfile::Dev,
            Some("network,recovery,presentation,-input"),
        );
        assert!(dimensions.contains(TraceDimension::Network));
        assert!(dimensions.contains(TraceDimension::Recovery));
        assert!(dimensions.contains(TraceDimension::Presentation));
        assert!(!dimensions.contains(TraceDimension::Input));
        assert!(!dimensions.contains(TraceDimension::Core));
    }

    #[test]
    fn production_records_key_but_drops_debug_rows() {
        let dimensions = TraceDimensionSet::default_for_profile(RuntimeTraceProfile::Production);
        let key = TraceMetadata::new(TraceDimension::Recovery, TraceImportance::Key);
        let debug = TraceMetadata::new(TraceDimension::Recovery, TraceImportance::Debug);
        assert!(should_record_trace_event(
            RuntimeTraceProfile::Production,
            dimensions,
            key,
            "xbxengine",
            "recoveryDecisionLedger",
            0.0,
        ));
        assert!(!should_record_trace_event(
            RuntimeTraceProfile::Production,
            dimensions,
            debug,
            "xbxengine",
            "runtimeEventRaw",
            0.0,
        ));
    }

    #[test]
    fn error_log_is_essential_even_when_engine_log_dimension_is_disabled() {
        let metadata = default_trace_metadata(
            "log",
            "xbxengine",
            "runtimeLog",
            &json!({ "level": "ERROR" }),
        );
        let dimensions = TraceDimensionSet::default_for_profile(RuntimeTraceProfile::Production);
        assert_eq!(metadata.importance, TraceImportance::Essential);
        assert!(should_record_trace_event(
            RuntimeTraceProfile::Production,
            dimensions,
            metadata,
            "xbxengine",
            "runtimeLog",
            0.0,
        ));
    }
}
