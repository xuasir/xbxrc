//! 会话控制面 trace 的有界聚合；轮询执行沿用 flow 的原时钟。
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Default)]
struct ControlWindow {
    facts: Value,
    last_emitted: Option<Value>,
    last_at_ms: u64,
    polls: u64,
    failures: u64,
    poll_started: Option<Instant>,
    duration_ms: u64,
}

#[derive(Default)]
pub(super) struct SessionControlTraces {
    sessions: HashMap<String, ControlWindow>,
}

impl SessionControlTraces {
    pub fn poll_started(&mut self, id: &str) {
        if self.sessions.len() < 256 || self.sessions.contains_key(id) {
            self.sessions.entry(id.to_owned()).or_default().poll_started = Some(Instant::now());
        }
    }

    pub fn poll_finished(
        &mut self,
        id: &str,
        state: Option<&str>,
        failed: bool,
        status: Option<u16>,
        at_ms: u64,
    ) -> Option<Value> {
        let entry = self.sessions.get_mut(id)?;
        entry.polls += 1;
        entry.failures += u64::from(failed);
        if let Some(started) = entry.poll_started.take() {
            entry.duration_ms += started.elapsed().as_millis() as u64;
        }
        if !entry.facts.is_object() {
            entry.facts = json!({});
        }
        entry.facts["streamState"] = json!(state.unwrap_or("unknown"));
        entry.facts["errorCode"] = json!(status);
        if failed {
            entry.facts["errorKind"] = json!("pollFailed");
            Self::publish(entry, at_ms, "pollFailed", false)
        } else {
            None
        }
    }

    pub fn observe(&mut self, id: &str, facts: Value, at_ms: u64) -> Option<Value> {
        if self.sessions.len() >= 256 && !self.sessions.contains_key(id) {
            return None;
        }
        let entry = self.sessions.entry(id.to_owned()).or_default();
        entry.facts = facts;
        let reason = if entry.last_emitted.is_none() {
            "first"
        } else if entry.last_emitted.as_ref() != Some(&entry.facts) {
            "changed"
        } else if at_ms.saturating_sub(entry.last_at_ms) >= 15_000 {
            "heartbeat"
        } else {
            return None;
        };
        Self::publish(entry, at_ms, reason, false)
    }

    pub fn finish(&mut self, id: &str, at_ms: u64) -> Option<Value> {
        let mut entry = self.sessions.remove(id)?;
        Self::publish(&mut entry, at_ms, "ended", true)
    }

    fn publish(entry: &mut ControlWindow, at_ms: u64, reason: &str, ended: bool) -> Option<Value> {
        let mut payload = entry.facts.clone();
        if !payload.is_object() {
            payload = json!({});
        }
        payload["reason"] = json!(reason);
        payload["ended"] = json!(ended);
        payload["intervalMs"] = json!(if entry.last_emitted.is_none() {
            0
        } else {
            at_ms.saturating_sub(entry.last_at_ms)
        });
        payload["pollCount"] = json!(entry.polls);
        payload["pollFailures"] = json!(entry.failures);
        payload["durationMs"] = json!(entry.duration_ms);
        entry.last_emitted = Some(entry.facts.clone());
        entry.last_at_ms = at_ms;
        Some(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_polls_keep_state_changes_heartbeats_and_final_counters() {
        let mut trace = SessionControlTraces::default();
        let mut rows = Vec::new();
        for at in (0..205000).step_by(2000) {
            trace.poll_started("session");
            trace.poll_finished("session", Some("ReadyToConnect"), false, None, at);
            let facts = json!({ "streamState": "ReadyToConnect", "phase": if at < 4000 { "connecting" } else { "connected" }, "playerState": "ready" });
            if let Some(row) = trace.observe("session", facts, at) {
                rows.push(row);
            }
        }
        assert_eq!(
            rows.iter().filter(|row| row["reason"] == "changed").count(),
            1
        );
        assert!(rows.len() <= 15);
        let ended = trace.finish("session", 205000).unwrap();
        assert_eq!(ended["pollCount"], 103);
        assert_eq!(ended["ended"], true);
        assert!(trace.sessions.is_empty());
    }
    #[test]
    fn failed_poll_is_immediate_and_preserves_failure_count() {
        let mut trace = SessionControlTraces::default();
        trace.poll_started("session");
        let row = trace
            .poll_finished("session", None, true, Some(401), 1)
            .unwrap();
        assert_eq!(row["errorCode"], 401);
        assert_eq!(row["pollFailures"], 1);
        assert_eq!(trace.finish("session", 2).unwrap()["pollFailures"], 1);
    }
}
