//! 流式写入用户选择的 ZIP；导出内容保持原有 trace 顺序和确认截止点。
use super::policy::RuntimeTraceProfile;
use super::runtime_state::TraceSegment;
use super::types::TraceFlushStatus;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

struct Source {
    file: File,
    length: u64,
}
fn sources(root: &Path, current: Vec<TraceSegment>, run: &str) -> Vec<Source> {
    let mut files: Vec<(u64, PathBuf)> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("runtime-trace-") && name.ends_with(".jsonl"))
        })
        .filter_map(|entry| {
            let mut reader = BufReader::new(File::open(entry.path()).ok()?.take(65537));
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;
            let header: Value = serde_json::from_str(&line).ok()?;
            (header["traceRunId"].as_str() == Some(run)).then(|| {
                (
                    header["payload"]["fileSequence"].as_u64().unwrap_or(0),
                    entry.path(),
                )
            })
        })
        .collect();
    // 当前 writer 已知片段也纳入，用于声明旋转过程中丢失的文件。
    for segment in current {
        if !files.iter().any(|(_, path)| path == &segment.path) && segment.path.exists() {
            let Ok(file) = File::open(&segment.path) else {
                continue;
            };
            let mut reader = BufReader::new(file.take(65537));
            let mut line = String::new();
            if reader.read_line(&mut line).is_ok()
                && serde_json::from_str::<Value>(&line)
                    .ok()
                    .is_some_and(|row| row["traceRunId"].as_str() == Some(run))
            {
                files.push((segment.file_sequence, segment.path));
            }
        }
    }
    files.sort_by_key(|(sequence, _)| *sequence);
    // 提前打开句柄并固定长度，两个遍历共享快照，轮转后继续读取已打开文件。
    files
        .into_iter()
        .filter_map(|(_, path)| {
            let file = File::open(path).ok()?;
            let length = file.metadata().ok()?.len();
            Some(Source { file, length })
        })
        .collect()
}

fn selected(row: &Value, run: &str, from: u128, to: u128, cutoff: Option<u64>) -> bool {
    row["traceRunId"].as_str() == Some(run)
        && row["seq"]
            .as_u64()
            .is_some_and(|seq| cutoff.is_none_or(|cutoff| seq <= cutoff))
        && row["tsMs"].as_u64().is_some_and(|at| {
            let at = at as u128;
            (at >= from && at <= to) || preamble(row, from)
        })
}

fn preamble(row: &Value, from: u128) -> bool {
    row["tsMs"].as_u64().is_some_and(|at| (at as u128) < from)
        && matches!(
            row["event"].as_str(),
            Some(
                "fileOpened"
                    | "runtimeBuildInfo"
                    | "playbackObservationManifest"
                    | "diagnosticCaptureStarted"
                    | "sessionVideoConstraintsResolved"
            )
        )
}
fn visit(
    files: &[Source],
    mut consume: impl FnMut(&str, &Value) -> io::Result<()>,
) -> io::Result<u64> {
    let mut invalid = 0;
    for source in files {
        let mut file = source.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let mut reader = BufReader::new(file.take(source.length));
        loop {
            let mut bytes = Vec::new();
            let read = (&mut reader).take(65537).read_until(b'\n', &mut bytes)?;
            if read == 0 {
                break;
            }
            if read > 65536 {
                if bytes.last() != Some(&b'\n') {
                    reader.skip_until(b'\n')?;
                }
                invalid += 1;
                continue;
            }
            let Ok(line) = String::from_utf8(bytes) else {
                invalid += 1;
                continue;
            };
            match serde_json::from_str::<Value>(&line) {
                Ok(row) => consume(line.trim_end(), &row)?,
                Err(_) => invalid += 1,
            }
        }
    }
    Ok(invalid)
}

fn safe_value(key: &str, value: &Value, depth: usize) -> Option<Value> {
    let lower = key.to_ascii_lowercase();
    if depth > 12
        || [
            "token",
            "authorization",
            "credential",
            "password",
            "secret",
            "cookie",
            "xuid",
            "gamertag",
            "handle",
            "sdp",
            "address",
            "url",
            "uri",
            "path",
            "filename",
            "titleid",
            "deviceid",
            "consoleid",
            "account",
            "username",
            "displayname",
        ]
        .iter()
        .any(|part| lower.contains(part))
    {
        return None;
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(text) => {
            let enums = [
                "active",
                "completed",
                "interrupted",
                "development",
                "production",
                "dev",
                "off",
                "flushed",
                "timeout",
                "ioError",
                "queueFull",
                "writerUnavailable",
                "start",
                "upgrade",
                "end",
                "failed",
                "blocked",
                "hardFault",
                "ok",
                "ready",
                "stable",
                "recovering",
                "available",
                "unavailable",
                "monotonic",
                "wall",
                "browser",
                "native",
                "request",
                "media",
                "input",
                "debug",
                "release",
                "main",
                "rust",
            ];
            let numeric = matches!(
                key,
                "sampleId"
                    | "firstSampleId"
                    | "lastSampleId"
                    | "episodeId"
                    | "operationId"
                    | "issueId"
                    | "captureId"
                    | "id"
                    | "buildTimestampUnixMs"
            ) && text.len() <= 64
                && text
                    .bytes()
                    .all(|ch| ch.is_ascii_digit() || matches!(ch, b':' | b'-'));
            let fingerprint = matches!(key, "gitCommitShort" | "sourceFingerprintSha256")
                && text.len() <= 64
                && text.bytes().all(|ch| ch.is_ascii_hexdigit());
            if enums.contains(&text.as_str()) || numeric || fingerprint {
                Some(value.clone())
            } else {
                None
            }
        }
        Value::Array(values) => Some(Value::Array(
            values
                .iter()
                .take(96)
                .filter_map(|value| safe_value(key, value, depth + 1))
                .collect(),
        )),
        Value::Object(values) => Some(Value::Object(
            values
                .iter()
                .filter(|(key, _)| {
                    key.len() <= 64
                        && key
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                })
                .filter_map(|(key, value)| {
                    safe_value(key, value, depth + 1).map(|value| (key.clone(), value))
                })
                .collect(),
        )),
    }
}

fn project(row: &Value, sessions: &mut HashMap<String, String>) -> Value {
    let mut safe = serde_json::Map::new();
    for key in ["schemaVersion", "seq", "sourceSequence", "tsMs"] {
        if row[key].is_number() {
            safe.insert(key.into(), row[key].clone());
        }
    }
    for key in [
        "traceRunId",
        "traceMode",
        "traceProfile",
        "dimension",
        "importance",
        "category",
        "domain",
        "event",
    ] {
        if let Some(text) = row[key].as_str().filter(|text| {
            text.len() <= 96
                && text
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.'))
        }) {
            safe.insert(key.into(), json!(text));
        }
    }
    let session = row["sessionId"].as_str().map(|id| {
        let next = format!("session-{}", sessions.len() + 1);
        sessions.entry(id.into()).or_insert(next).clone()
    });
    safe.insert("sessionId".into(), json!(session));
    let event = row["event"].as_str().unwrap_or_default();
    let payload = if super::trace_contract::contract(event).is_some() {
        super::trace_contract::project_payload(event, &row["payload"])
    } else {
        safe_value("payload", &row["payload"], 0).unwrap_or_else(|| json!({}))
    };
    safe.insert("payload".into(), payload);
    safe.into()
}

fn write_json<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    value: &Value,
) -> io::Result<()> {
    zip.start_file(name, SimpleFileOptions::default())?;
    serde_json::to_writer(&mut *zip, value)?;
    zip.write_all(b"\n")
}

pub(super) fn safe_configuration(configuration: &Value) -> Value {
    let mut safe = serde_json::Map::new();
    for key in [
        "resolution",
        "xhome_resolution",
        "xhome_bitrate",
        "xcloud_bitrate",
        "audio_bitrate",
        "ipv6",
        "enable_audio_control",
        "prevent_resolution_drops",
        "background_keepalive",
        "diagnostics_lightweight",
    ] {
        if let Some(value) = configuration
            .get(key)
            .filter(|value| value.is_number() || value.is_boolean())
        {
            safe.insert(key.into(), value.clone());
        }
    }
    if let Some(display) = configuration["display_options"].as_object() {
        let display: serde_json::Map<String, Value> = display
            .iter()
            .filter(|(key, value)| {
                matches!(
                    key.as_str(),
                    "sharpness" | "saturation" | "contrast" | "brightness"
                ) && value.is_number()
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        safe.insert("display_options".into(), Value::Object(display));
    }
    Value::Object(safe)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn save(
    path: &Path,
    root: &Path,
    current: Vec<TraceSegment>,
    context: &(String, u128, Option<u128>, Value),
    cutoff: u64,
    flush: TraceFlushStatus,
    status: Value,
    configuration: Value,
    profile: RuntimeTraceProfile,
) -> io::Result<()> {
    let record_id = context.3["issue"]["id"]
        .as_str()
        .or_else(|| context.3["capture"]["id"].as_str());
    let record_path = record_id
        .filter(|id| id.bytes().all(|ch| ch.is_ascii_digit() || ch == b'-'))
        .map(|id| {
            root.join("diagnostic-records")
                .join(format!("diagnostic-record-{id}.jsonl"))
        });
    let mut context = context.clone();
    let files = if let Some(file) = record_path.and_then(|path| File::open(path).ok()) {
        let length = file.metadata()?.len();
        context.3["sequenceScope"] = json!("diagnostic-record");
        vec![Source { file, length }]
    } else {
        sources(root, current, &context.0)
    };
    let limit = if profile == RuntimeTraceProfile::Dev {
        80 * 1024 * 1024
    } else {
        32 * 1024 * 1024
    };
    save_snapshot(
        path,
        &files,
        &context,
        cutoff,
        flush,
        status,
        configuration,
        limit,
    )
}

#[allow(clippy::too_many_arguments)]
fn save_snapshot(
    path: &Path,
    files: &[Source],
    context: &(String, u128, Option<u128>, Value),
    cutoff: u64,
    flush: TraceFlushStatus,
    status: Value,
    configuration: Value,
    limit: usize,
) -> io::Result<()> {
    let to = context.2.unwrap_or_else(super::service::now_ms);
    let same_run = status["traceRunId"].as_str() == Some(&context.0);
    let cutoff = (same_run && context.3["sequenceScope"] != "diagnostic-record").then_some(cutoff);
    let limit = limit.saturating_sub(256 * 1024);
    let mut total = 0usize;
    let mut context_bytes = 0usize;
    let mut sessions = HashMap::new();
    let mut source_mode = Value::Null;
    let mut source_build = Value::Null;
    let invalid = visit(files, |_, row| {
        if selected(row, &context.0, context.1, to, cutoff) {
            let safe = project(row, &mut sessions);
            let size = serde_json::to_vec(&safe)?.len() + 1;
            total = total.saturating_add(size);
            if preamble(row, context.1) {
                context_bytes = context_bytes.saturating_add(size);
            }
            if row["event"] == "fileOpened" {
                source_mode = safe["traceProfile"].clone();
                if safe["payload"]["buildFingerprint"].is_object() {
                    source_build = json!({"buildFingerprint":safe["payload"]["buildFingerprint"]});
                }
            }
            if row["event"] == "runtimeBuildInfo" {
                source_build = safe["payload"].clone();
            }
        }
        Ok(())
    })?;
    if total == 0 {
        return Err(io::Error::other(
            "diagnostic records are no longer available",
        ));
    }
    let temporary = path.with_file_name(format!(
        ".xbxrc-diagnostics-{}.partial",
        super::service::now_ms()
    ));
    let result = (|| -> io::Result<()> {
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("runtime-trace.jsonl", SimpleFileOptions::default())?;
        let mut omit = total.saturating_sub(limit);
        let mut rows = 0u64;
        let mut first = None;
        let mut last = None;
        let mut written = 0usize;
        visit(files, |_, row| {
            if !selected(row, &context.0, context.1, to, cutoff) {
                return Ok(());
            }
            let line = serde_json::to_string(&project(row, &mut sessions))?;
            if omit > 0 && !(context_bytes <= limit / 4 && preamble(row, context.1)) {
                omit = omit.saturating_sub(line.len() + 1);
                return Ok(());
            }
            if written + line.len() + 1 > limit {
                return Ok(());
            }
            zip.write_all(line.as_bytes())?;
            zip.write_all(b"\n")?;
            written += line.len() + 1;
            rows += 1;
            first.get_or_insert(row["seq"].as_u64());
            last = row["seq"].as_u64();
            Ok(())
        })?;
        if rows == 0 {
            return Err(io::Error::other(
                "diagnostic export has no available records",
            ));
        }
        let manifest = json!({"bundleVersion":1,"schemaVersion":3,"productionContractVersion":1,
            "traceRunId":context.0,"fromMs":context.1,"toMs":to,"selection":context.3,"cutoffSequence":cutoff,
            "flushStatus":if same_run {json!(flush)} else {Value::Null},"rowCount":rows,"firstSequence":first.flatten(),"lastSequence":last,
            "invalidOrMissingRows":invalid,"rangeTruncated":total>limit,"traceBytes":written,
            "sourceFileCount":files.len(),"rotationCoverage":"retained files only; previous files may already be rotated",
            "sourceMode":source_mode,"sourceBuild":source_build,"writer":if same_run {safe_value("writer",&status["backend"],0)} else {None},
            "exporter":{"version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"mode":status["diagnostics"]["effectiveMode"],"diagnostics":safe_value("diagnostics",&status["diagnostics"],0)},
            "configurationScope":"export-time configuration","privacy":"local session mapping; core projection; undefined text and identifying keys omitted",
            "healthRetention":"sparse checkpoints with bounded detailed context; per-row window coverage is authoritative"});
        write_json(&mut zip, "manifest.json", &manifest)?;
        write_json(
            &mut zip,
            "config-snapshot.json",
            &safe_configuration(&configuration),
        )?;
        zip.start_file("summary.md", SimpleFileOptions::default())?;
        writeln!(zip, "# XBXRC 诊断记录\n\n范围：{}–{} ms；{} 行；trace schema v3。\n\n采集与问题引用见 manifest.json；健康记录按轻量策略保留，逐条 window/coverage 表达实际证据范围。网络、恢复、media supply、steady supply、提交/呈现长尾与帧供给 delta 沿原始字段分析。\n\n截断：{}；无效或缺失记录：{}；flush：{:?}。",context.1,to,rows,total>limit,invalid,flush)?;
        let file = zip.finish()?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_projects_only_numeric_and_boolean_product_values() {
        let safe = safe_configuration(
            &json!({"resolution":1080,"server_credential":"secret","display_options":{"sharpness":1,"token":"secret"},"ipv6":true}),
        );
        assert_eq!(safe["resolution"], 1080);
        assert!(safe.get("server_credential").is_none());
        assert!(safe["display_options"].get("token").is_none());
    }
    #[test]
    fn export_projection_removes_raw_text_identities_and_paths() {
        let row = project(
            &json!({"traceRunId":"run","seq":1,"tsMs":1,"event":"rawOffer","sessionId":"private-handle",
            "payload":{"sdp":"private","url":"https://private","xuid":123,"token":"secret","path":"/Users/alice","message":"Bearer hidden","durationMs":42,"phase":"failed"}}),
            &mut HashMap::new(),
        );
        assert_eq!(row["sessionId"], "session-1");
        assert_eq!(row["payload"], json!({"durationMs":42,"phase":"failed"}));
        let writer = safe_value(
            "writer",
            &json!({"files":[{"path":"/Users/alice"}],"writtenRows":10}),
            0,
        )
        .unwrap();
        assert!(!writer.to_string().contains("alice"));
    }
    #[test]
    fn rotated_snapshot_retains_order_range_cutoff_and_source_identity() {
        let root = std::env::temp_dir().join(format!("diag-export-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("runtime-trace-test.jsonl");
        let rows = [
            json!({"traceRunId":"old","seq":1,"tsMs":10,"event":"fileOpened","traceProfile":"production","payload":{"fileSequence":1}}),
            json!({"traceRunId":"old","seq":2,"tsMs":20,"event":"runtimeBuildInfo","payload":{"buildFingerprint":{"gitCommitShort":"abc123"}}}),
            json!({"traceRunId":"old","seq":3,"tsMs":100,"event":"event","payload":{"durationMs":42}}),
            json!({"traceRunId":"old","seq":4,"tsMs":200,"event":"event","payload":{"durationMs":50}}),
        ];
        fs::write(
            &log,
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let snapshot = sources(&root, Vec::new(), "old");
        fs::rename(&log, root.join("rotated")).unwrap();
        fs::write(&log, "private\n").unwrap();
        let out = root.join("out.zip");
        save_snapshot(
            &out,
            &snapshot,
            &("old".into(), 90, Some(150), json!({})),
            3,
            TraceFlushStatus::Flushed,
            json!({"traceRunId":"new","backend":{"files":[{"path":"/Users/alice"}]}}),
            json!({}),
            1024 * 1024,
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let mut trace = String::new();
        zip.by_name("runtime-trace.jsonl")
            .unwrap()
            .read_to_string(&mut trace)
            .unwrap();
        assert_eq!(trace.lines().count(), 3);
        let mut manifest = String::new();
        zip.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        let manifest: Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest["sourceMode"], "production");
        assert_eq!(
            manifest["sourceBuild"]["buildFingerprint"]["gitCommitShort"],
            "abc123"
        );
        assert!(manifest["cutoffSequence"].is_null());
        assert!(manifest["writer"].is_null());
        drop(zip);
        drop(snapshot);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn truncated_bundle_keeps_recent_ordered_rows_and_empty_selection_fails() {
        let root = std::env::temp_dir().join(format!("diag-budget-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("runtime-trace-test.jsonl");
        fs::write(&log,(1..=100).map(|seq|format!("{}\n",json!({"traceRunId":"run","seq":seq,"tsMs":seq,"event":"event","payload":{"values":vec![123;20]}}))).collect::<String>()).unwrap();
        let snapshot = sources(&root, Vec::new(), "run");
        let out = root.join("out.zip");
        save_snapshot(
            &out,
            &snapshot,
            &("run".into(), 0, Some(100), json!({})),
            99,
            TraceFlushStatus::Flushed,
            json!({"traceRunId":"run"}),
            json!({}),
            256 * 1024 + 4096,
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let mut trace = String::new();
        zip.by_name("runtime-trace.jsonl")
            .unwrap()
            .read_to_string(&mut trace)
            .unwrap();
        assert!(trace.len() <= 4096);
        let rows: Vec<Value> = trace
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.last().unwrap()["seq"], 99);
        assert!(rows
            .windows(2)
            .all(|rows| rows[0]["seq"].as_u64() < rows[1]["seq"].as_u64()));
        assert!(save_snapshot(
            &root.join("empty.zip"),
            &snapshot,
            &("missing".into(), 0, Some(100), json!({})),
            99,
            TraceFlushStatus::Flushed,
            json!({}),
            json!({}),
            1024 * 1024
        )
        .is_err());
        drop(zip);
        drop(snapshot);
        fs::remove_dir_all(root).unwrap();
    }
}
