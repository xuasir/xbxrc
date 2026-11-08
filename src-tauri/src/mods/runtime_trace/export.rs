//! 导出同一 run 的有序保留片段；使用已确认 flush 的 sequence 截止点。
use super::runtime_state::TraceSegment;
use super::types::TraceFlushStatus;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, BufRead, BufReader};

const MAX_EXPORT_BYTES: usize = 80 * 1024 * 1024;

pub(super) fn export_bundle(
    run_id: &str,
    segments: Vec<TraceSegment>,
    cutoff: u64,
    status: TraceFlushStatus,
) -> io::Result<Value> {
    let mut body = Vec::new();
    let mut files = Vec::new();
    let mut missing_files = 0;
    let mut truncated_rows = 0;
    let mut row_count = 0;
    for segment in segments {
        let file = match File::open(&segment.path) {
            Ok(file) => file,
            Err(_) => {
                missing_files += 1;
                continue;
            }
        };
        let mut first = None;
        let mut last = None;
        for line in BufReader::new(file).lines() {
            let line = line?;
            let row = match serde_json::from_str::<Value>(&line) {
                Ok(row) => row,
                Err(_) => {
                    truncated_rows += 1;
                    continue;
                }
            };
            let Some(seq) = row["seq"].as_u64() else {
                continue;
            };
            if seq > cutoff || row["traceRunId"].as_str() != Some(run_id) {
                continue;
            }
            if body.len() + line.len() + 1 > MAX_EXPORT_BYTES {
                return Err(io::Error::other("runtime trace export exceeds 80MiB"));
            }
            body.extend_from_slice(line.as_bytes());
            body.push(b'\n');
            first.get_or_insert(seq);
            last = Some(seq);
            row_count += 1;
        }
        files.push(json!({ "fileName": segment.file_name, "fileSequence": segment.file_sequence, "firstSequence": first, "lastSequence": last }));
    }
    let manifest = json!({ "productionContractVersion": 1, "traceRunId": run_id, "files": files, "cutoffSequence": cutoff,
        "flushStatus": status, "missingFiles": missing_files, "truncatedRows": truncated_rows, "rowCount": row_count,
        "partialBeginning": files.first().is_some_and(|file| file["firstSequence"].as_u64().unwrap_or(0) > 1) });
    let mut bytes = serde_json::to_vec(
        &json!({ "schemaVersion": 3, "traceRunId": run_id, "event": "traceExportManifest", "payload": manifest }),
    )?;
    bytes.push(b'\n');
    bytes.extend(body);
    Ok(
        json!({ "suggestedName": format!("runtime-trace-run-{run_id}.jsonl"), "dataBase64": STANDARD.encode(bytes), "manifest": manifest }),
    )
}
