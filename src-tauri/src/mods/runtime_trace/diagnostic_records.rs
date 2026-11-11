//! 问题窗口由现有 trace writer 同步保留，独立于常规日志轮转。
use super::diagnostics_state::{MAX_RECORDS, MAX_RECORD_BYTES};
use super::service::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
pub(super) struct RecordWindow {
    pub id: String,
    pub from: u128,
    pub to: u128,
    pub manual: bool,
}
struct Window {
    id: String,
    from: u128,
    to: u128,
    deadline: Instant,
    file: BufWriter<File>,
    bytes: usize,
    limit: usize,
    truncated: bool,
    sequence: u64,
}
pub(super) struct RecordFiles {
    root: PathBuf,
    run: String,
    windows: Vec<Window>,
    last_prune: Instant,
}
impl RecordFiles {
    pub fn new(root: PathBuf, run: String) -> Self {
        let mut records = Self {
            root: root.join("diagnostic-records"),
            run,
            windows: Vec::new(),
            last_prune: Instant::now(),
        };
        let _ = records.prune();
        records
    }
    pub fn open(&mut self, window: RecordWindow, sequence: u64, profile: &str) -> io::Result<()> {
        if window.id.is_empty()
            || !window
                .id
                .bytes()
                .all(|ch| ch.is_ascii_digit() || ch == b'-')
        {
            return Err(io::Error::other("invalid record id"));
        }
        if let Some(active) = self
            .windows
            .iter_mut()
            .find(|active| active.id == window.id)
        {
            active.to = active.to.max(window.to);
            active.deadline = Instant::now()
                + Duration::from_millis(active.to.saturating_sub(now_ms()).min(600000) as u64);
            return Ok(());
        }
        self.flush()?;
        if self.windows.len() >= 8 {
            return Err(io::Error::other("diagnostic window capacity reached"));
        }
        fs::create_dir_all(&self.root)?;
        let path = self
            .root
            .join(format!("diagnostic-record-{}.jsonl", window.id));
        // 重复错误延续同一窗口；记录序号独立于常规日志序号。
        let existing = path.exists();
        let mut file = BufWriter::new(OpenOptions::new().append(true).create(true).open(&path)?);
        let header = json!({"schemaVersion":3,"seq":0,"sourceSequence":sequence,"traceRunId":self.run,"tsMs":now_ms(),"traceProfile":profile,
            "dimension":"core","importance":"essential","category":"state","domain":"trace","event":"fileOpened","sessionId":null,
            "payload":{"fileSequence":0,"recordId":window.id,"fromMs":window.from,"toMs":window.to,"manual":window.manual,"buildFingerprint":crate::build_info::current_build_fingerprint()}});
        let bytes = serde_json::to_vec(&header)?;
        if !existing {
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            file.flush()?;
        }
        let written = file.get_ref().metadata()?.len() as usize;
        let previous = if existing {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };
        let record_sequence = previous.lines().count().saturating_sub(1) as u64;
        let truncated = previous.contains("diagnosticRecordTruncated");
        self.windows.push(Window {
            id: window.id,
            from: window.from,
            to: window.to,
            deadline: Instant::now()
                + Duration::from_millis(window.to.saturating_sub(now_ms()).min(600000) as u64),
            file,
            bytes: written,
            limit: if window.manual {
                16 * 1024 * 1024
            } else {
                2 * 1024 * 1024
            },
            truncated,
            sequence: record_sequence,
        });
        self.prune()?;
        Ok(())
    }
    pub fn close(&mut self, id: &str) -> io::Result<()> {
        if let Some(index) = self.windows.iter().position(|window| window.id == id) {
            self.windows.remove(index).file.flush()?;
        }
        self.prune()?;
        Ok(())
    }
    pub fn append(&mut self, bytes: &[u8], sequence: u64) -> io::Result<()> {
        if self.windows.is_empty() {
            return Ok(());
        }
        let at = serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|row| row["tsMs"].as_u64())
            .unwrap_or(0) as u128;
        for window in &mut self.windows {
            if Instant::now() >= window.deadline
                || at < window.from
                || at > window.to
                || window.truncated
            {
                continue;
            }
            window.sequence += 1;
            let prefix = format!(
                "{{\"seq\":{},\"sourceSequence\":{sequence},",
                window.sequence
            );
            let size = prefix.len() + bytes.len() - 1;
            if window.bytes + size > window.limit - 1024 {
                window.truncated = true;
                let marker = json!({"schemaVersion":3,"seq":window.sequence,"sourceSequence":sequence,"traceRunId":self.run,"tsMs":now_ms(),"event":"diagnosticRecordTruncated","payload":{"recordLimitBytes":window.limit,"truncated":true}});
                writeln!(window.file, "{marker}")?;
                continue;
            }
            window.file.write_all(prefix.as_bytes())?;
            window.file.write_all(&bytes[1..])?;
            window.bytes += size;
        }
        Ok(())
    }
    pub fn flush(&mut self) -> io::Result<()> {
        let had_windows = !self.windows.is_empty();
        for window in &mut self.windows {
            window.file.flush()?;
        }
        self.windows
            .retain(|window| Instant::now() < window.deadline);
        if had_windows
            && (self.windows.is_empty() || self.last_prune.elapsed() >= Duration::from_secs(2))
        {
            self.prune()?;
            self.last_prune = Instant::now();
        }
        Ok(())
    }
    fn prune(&mut self) -> io::Result<()> {
        let mut files = fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    name.starts_with("diagnostic-record-") && name.ends_with(".jsonl")
                })
            })
            .filter_map(|entry| {
                entry
                    .metadata()
                    .ok()
                    .map(|metadata| (metadata.modified().ok(), metadata.len(), entry.path()))
            })
            .collect::<Vec<_>>();
        files.sort_by_key(|file| file.0);
        let mut count = files.len();
        let mut total = files.iter().map(|file| file.1).sum::<u64>();
        for (_, bytes, path) in files {
            if count <= MAX_RECORDS && total <= MAX_RECORD_BYTES {
                break;
            }
            if self.windows.iter().any(|window| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy() == format!("diagnostic-record-{}.jsonl", window.id)
                })
            }) {
                continue;
            }
            if fs::remove_file(path).is_ok() {
                count -= 1;
                total = total.saturating_sub(bytes);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_expire_without_ui_and_capacity_and_storage_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let mut records = RecordFiles::new(root.path().into(), "run".into());
        for id in 1..=8 {
            records
                .open(
                    RecordWindow {
                        id: id.to_string(),
                        from: 0,
                        to: now_ms() + 10000,
                        manual: false,
                    },
                    0,
                    "production",
                )
                .unwrap();
        }
        assert!(records
            .open(
                RecordWindow {
                    id: "9".into(),
                    from: 0,
                    to: now_ms() + 10000,
                    manual: false
                },
                0,
                "production"
            )
            .is_err());
        records.windows[0].limit = 2048;
        let row = serde_json::to_vec(
            &json!({"tsMs":now_ms(),"event":"health","payload":{"values":vec![1234;1000]}}),
        )
        .unwrap();
        records.append(&row, 1).unwrap();
        records.flush().unwrap();
        let data = fs::read_to_string(records.root.join("diagnostic-record-1.jsonl")).unwrap();
        assert!(data.contains("diagnosticRecordTruncated"));
        assert!(data.len() < 2048);
        for window in &mut records.windows {
            window.deadline = Instant::now() - Duration::from_secs(1);
        }
        records.flush().unwrap();
        assert!(records.windows.is_empty());
        for id in 9..=35 {
            fs::write(
                records.root.join(format!("diagnostic-record-{id}.jsonl")),
                "{}\n",
            )
            .unwrap();
        }
        let restored = RecordFiles::new(root.path().into(), "next-run".into());
        assert_eq!(fs::read_dir(&restored.root).unwrap().count(), 12);
    }
}
