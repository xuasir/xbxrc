//! 诊断采集与轻量上下文的有界运行态；时间与保留范围由 Rust 持有。
use super::policy::TraceImportance;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

pub(super) const CONTEXT_MS: u128 = 10_000;
const MAX_CONTEXT_BYTES: usize = 256 * 1024;
const MAX_CONTEXT_ROWS: usize = 96;
pub(super) const MAX_RECORDS: usize = 12;
pub(super) const MAX_RECORD_BYTES: u64 = 64 * 1024 * 1024;
const CAPTURE_MS: u128 = 600_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticIssue {
    pub id: String,
    pub trace_run_id: String,
    pub source: String,
    pub operation: String,
    pub code: String,
    pub first_at_ms: u128,
    pub last_at_ms: u128,
    pub from_ms: u128,
    pub to_ms: u128,
    pub count: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCapture {
    pub id: String,
    pub trace_run_id: String,
    pub started_at_ms: u128,
    pub expires_at_ms: u128,
    pub ended_at_ms: Option<u128>,
    pub state: String,
}

pub(super) struct BufferedRow {
    pub in_trace: bool,
    pub at_ms: u128,
    pub bytes: Vec<u8>,
    pub generation: u64,
    pub importance: TraceImportance,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DiagnosticIndex {
    pub issues: VecDeque<DiagnosticIssue>,
    pub capture: Option<DiagnosticCapture>,
    #[serde(default)]
    pub captures: VecDeque<DiagnosticCapture>,
}

pub(super) struct DiagnosticState {
    pub lightweight: bool,
    pub index: DiagnosticIndex,
    pub buffer: VecDeque<BufferedRow>,
    pub buffered_bytes: usize,
    pub buffered_total: u64,
    pub context_dropped: u64,
    pub replay_dropped: u64,
    detail_until: Option<Instant>,
    capture_deadline: Option<Instant>,
    sampled: HashMap<String, Instant>,
    next_id: u64,
    detail_minute: Instant,
    detail_windows: u32,
}

impl DiagnosticState {
    pub fn new(mut index: DiagnosticIndex) -> Self {
        index.issues.truncate(MAX_RECORDS);
        if let Some(capture) = index.capture.as_mut() {
            if capture.state == "active" {
                capture.state = "interrupted".into();
            }
        }
        for capture in &mut index.captures {
            if capture.state == "active" {
                capture.state = "interrupted".into();
            }
        }
        index.captures.truncate(MAX_RECORDS);
        trim_history(&mut index);
        Self {
            lightweight: false,
            index,
            buffer: VecDeque::new(),
            buffered_bytes: 0,
            buffered_total: 0,
            context_dropped: 0,
            replay_dropped: 0,
            detail_until: None,
            capture_deadline: None,
            sampled: HashMap::new(),
            next_id: 0,
            detail_minute: Instant::now(),
            detail_windows: 0,
        }
    }

    pub fn expire(&mut self, at: u128) -> bool {
        if self
            .capture_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.stop(at);
            return true;
        }
        false
    }

    pub fn active(&self) -> bool {
        self.capture_deadline
            .is_some_and(|deadline| Instant::now() < deadline)
    }

    pub fn detailed(&self) -> bool {
        self.active()
            || self
                .detail_until
                .is_some_and(|deadline| Instant::now() < deadline)
    }

    pub fn start(&mut self, run: &str, at: u128) -> DiagnosticCapture {
        self.expire(at);
        if self.active() {
            return self.index.capture.clone().expect("active capture");
        }
        self.next_id += 1;
        let capture = DiagnosticCapture {
            id: format!("{at}-{}", self.next_id),
            trace_run_id: run.into(),
            started_at_ms: at,
            expires_at_ms: at + CAPTURE_MS,
            ended_at_ms: None,
            state: "active".into(),
        };
        self.index.capture = Some(capture.clone());
        self.index.captures.push_front(capture.clone());
        trim_history(&mut self.index);
        self.capture_deadline = Some(Instant::now() + Duration::from_millis(CAPTURE_MS as u64));
        capture
    }

    pub fn stop(&mut self, at: u128) -> Option<DiagnosticCapture> {
        self.capture_deadline = None;
        if let Some(capture) = self.index.capture.as_mut() {
            if capture.state == "active" {
                capture.ended_at_ms = Some(at.min(capture.expires_at_ms));
                capture.state = "completed".into();
            }
        }
        if let Some(capture) = self.index.capture.as_ref() {
            if let Some(record) = self
                .index
                .captures
                .iter_mut()
                .find(|record| record.id == capture.id)
            {
                *record = capture.clone();
            }
        }
        self.index.capture.clone()
    }

    pub fn issue(
        &mut self,
        run: &str,
        source: &str,
        operation: &str,
        code: &str,
        at: u128,
    ) -> DiagnosticIssue {
        if let Some(issue) = self.index.issues.iter_mut().find(|issue| {
            issue.trace_run_id == run
                && issue.source == source
                && issue.operation == operation
                && issue.code == code
                && at.saturating_sub(issue.first_at_ms) < 30_000
        }) {
            issue.last_at_ms = at;
            issue.to_ms = (at + CONTEXT_MS).min(issue.first_at_ms + 40_000);
            self.detail_until =
                Some(Instant::now() + Duration::from_millis(issue.to_ms.saturating_sub(at) as u64));
            issue.count += 1;
            return issue.clone();
        }
        self.next_id += 1;
        if self.detail_minute.elapsed() >= Duration::from_secs(60) {
            self.detail_minute = Instant::now();
            self.detail_windows = 0;
        }
        if self.detail_windows < 3
            && self
                .detail_until
                .is_none_or(|deadline| Instant::now() >= deadline)
        {
            self.detail_until = Some(Instant::now() + Duration::from_millis(CONTEXT_MS as u64));
            self.detail_windows += 1;
        }
        let issue = DiagnosticIssue {
            id: format!("{at}-{}", self.next_id),
            trace_run_id: run.into(),
            source: source.into(),
            operation: operation.into(),
            code: code.into(),
            first_at_ms: at,
            last_at_ms: at,
            from_ms: at.saturating_sub(CONTEXT_MS),
            to_ms: at + CONTEXT_MS,
            count: 1,
        };
        self.index.issues.push_front(issue.clone());
        self.index.issues.truncate(MAX_RECORDS);
        trim_history(&mut self.index);
        issue
    }

    /// 周期摘要的准入按会话/事件计时，生命周期与异常继续即时保留。
    pub fn sparse(&mut self, event: &str, session: Option<&str>) -> bool {
        let key = format!("{event}:{}", session.unwrap_or_default());
        let now = Instant::now();
        if self.sampled.get(&key).is_some_and(|before| {
            now.duration_since(*before) < Duration::from_secs(if self.lightweight { 30 } else { 5 })
        }) {
            return true;
        }
        if self.sampled.len() >= 32 {
            self.sampled.clear();
        }
        self.sampled.insert(key, now);
        false
    }

    pub fn buffer(&mut self, row: BufferedRow, at: u128) {
        self.prune(at);
        if row.bytes.len() > MAX_CONTEXT_BYTES {
            self.context_dropped += 1;
            return;
        }
        while self.buffer.len() >= MAX_CONTEXT_ROWS
            || self.buffered_bytes + row.bytes.len() > MAX_CONTEXT_BYTES
        {
            if let Some(old) = self.buffer.pop_front() {
                self.buffered_bytes -= old.bytes.len();
                self.context_dropped += 1;
            } else {
                break;
            }
        }
        self.buffered_bytes += row.bytes.len();
        self.buffered_total += 1;
        self.buffer.push_back(row);
    }

    pub fn take_context(&mut self, at: u128) -> Vec<BufferedRow> {
        self.prune(at);
        self.buffered_bytes = 0;
        self.buffer.drain(..).collect()
    }

    fn prune(&mut self, at: u128) {
        while self
            .buffer
            .front()
            .is_some_and(|row| at.saturating_sub(row.at_ms) > CONTEXT_MS)
        {
            if let Some(row) = self.buffer.pop_front() {
                self.buffered_bytes -= row.bytes.len();
            }
        }
    }
}

fn trim_history(index: &mut DiagnosticIndex) {
    while index.issues.len() + index.captures.len() > MAX_RECORDS {
        let capture = index
            .captures
            .iter()
            .rposition(|capture| capture.state != "active");
        let issue_at = index.issues.back().map(|issue| issue.first_at_ms);
        let capture_at = capture.map(|position| index.captures[position].started_at_ms);
        if capture_at.is_some_and(|at| issue_at.is_none_or(|issue_at| at < issue_at)) {
            index.captures.remove(capture.unwrap());
        } else {
            index.issues.pop_back();
        }
    }
}

pub(super) fn periodic(event: &str) -> bool {
    matches!(
        event,
        "browserPlaybackHealthObserved"
            | "decodedFrameSupplyObserved"
            | "inputUiHealthObserved"
            | "resolutionFeedbackObserved"
            | "homeConfigurationObserved"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_keeps_twelve_records_and_protects_active_capture() {
        let mut state = DiagnosticState::new(DiagnosticIndex::default());
        let capture = state.start("run", 100);
        for at in 1..=20 {
            state.issue("run", "request", "data.list", "DATA_ERROR", at * 50_000);
        }
        assert_eq!(state.index.issues.len() + state.index.captures.len(), 12);
        assert_eq!(state.index.captures[0].id, capture.id);
        assert_eq!(state.index.issues.back().unwrap().first_at_ms, 500_000);

        // 旧版本保存的较长索引在启动时收敛到同一保留上限。
        for at in 1..=20 {
            let mut old_capture = capture.clone();
            old_capture.id = format!("old-{at}");
            old_capture.started_at_ms = at;
            old_capture.state = "completed".into();
            state.index.captures.push_back(old_capture);
        }
        let restored = DiagnosticState::new(state.index);
        assert_eq!(
            restored.index.issues.len() + restored.index.captures.len(),
            12
        );
    }
    #[test]
    fn capture_is_idempotent_expires_and_previous_run_is_interrupted() {
        let mut state = DiagnosticState::new(DiagnosticIndex::default());
        let capture = state.start("run", 100);
        assert_eq!(capture.id, state.start("run", 101).id);
        assert_eq!(
            DiagnosticState::new(DiagnosticIndex {
                issues: VecDeque::new(),
                capture: Some(capture),
                ..Default::default()
            })
            .index
            .capture
            .unwrap()
            .state,
            "interrupted"
        );
        state.capture_deadline = Some(Instant::now() - Duration::from_secs(1));
        assert!(state.expire(700_000));
        assert!(!state.active());
        assert_eq!(state.index.capture.unwrap().ended_at_ms, Some(600_100));
    }
    #[test]
    fn duplicate_issues_merge_and_context_is_bounded() {
        let mut state = DiagnosticState::new(DiagnosticIndex::default());
        let issue = state.issue("run", "request", "data.list", "DATA_ERROR", 10000);
        let repeated = state.issue("run", "request", "data.list", "DATA_ERROR", 11000);
        assert_eq!(issue.id, repeated.id);
        assert_eq!(repeated.count, 2);
        for at in 0..200 {
            state.buffer(
                BufferedRow {
                    in_trace: false,
                    at_ms: at,
                    bytes: vec![0; 4096],
                    generation: 1,
                    importance: TraceImportance::Key,
                },
                at,
            );
        }
        assert!(state.buffered_bytes <= MAX_CONTEXT_BYTES);
        assert!(state.context_dropped > 0);
        assert!(state.take_context(20000).is_empty());
        assert_eq!(state.buffered_bytes, 0);
    }
    #[test]
    fn automatic_windows_are_bounded_and_capture_overrides_sparse_retention() {
        let mut state = DiagnosticState::new(DiagnosticIndex::default());
        assert!(!state.detailed());
        for at in 0..4 {
            state.issue("run", "media", "playback", "fault", at * 50000);
            assert_eq!(state.detail_windows, (at + 1).min(3) as u32);
            state.detail_until = Some(Instant::now() - Duration::from_secs(1));
        }
        assert!(!state.detailed());
        state.start("run", 500000);
        assert!(state.detailed());
        assert!(!state.sparse("health", Some("session")));
        assert!(state.sparse("health", Some("session")));
        assert!(!state.sparse("health", Some("another")));
    }
}
