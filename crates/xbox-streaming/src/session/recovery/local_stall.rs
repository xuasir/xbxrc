use super::{
    runtime_stall_threshold, RuntimeMediaCounters, RuntimeSupervisionSample, STABLE_WINDOW_MS,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLocalStallDecision {
    pub domain: String,
    pub action: String,
    pub age_ms: u64,
    pub coverage_ms: u64,
    pub eligible: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct Snapshot {
    at: u64,
    peer: u64,
    video: u64,
    renderer: Option<u64>,
    renderer_kind: Option<String>,
    counters: RuntimeMediaCounters,
    submitted: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LocalStallState {
    previous: Option<Snapshot>,
    since: Option<u64>,
    domain: Option<&'static str>,
    eligible_since: u64,
}

impl LocalStallState {
    pub fn observe(
        &mut self,
        sample: &RuntimeSupervisionSample,
        fresh: bool,
        duplicate: bool,
    ) -> Option<RuntimeLocalStallDecision> {
        if !fresh
            || !sample.has_presented_media
            || sample.now_ms < sample.warmup_until_ms
            || sample.connected_elapsed_ms < STABLE_WINDOW_MS
            || sample.connection_state != "connected"
            || sample.control_channel_state != "open"
        {
            *self = Self::default();
            return None;
        }
        if duplicate {
            return None;
        }
        if sample.eligible_since_ms > self.eligible_since {
            *self = Self {
                eligible_since: sample.eligible_since_ms,
                ..Self::default()
            };
        }
        if sample
            .sampled_at_ms
            .is_none_or(|at| at < self.eligible_since)
        {
            self.previous = None;
            return None;
        }
        let Some(counters) = sample.local_counters.as_ref() else {
            *self = Self::default();
            return None;
        };
        let current = Snapshot {
            at: sample.sampled_at_ms?,
            peer: sample.peer_generation,
            video: sample.video_generation,
            renderer: sample.renderer_generation,
            renderer_kind: sample.renderer_kind.clone(),
            counters: counters.clone(),
            submitted: sample.renderer_submitted_total,
        };
        let previous = self.previous.replace(current.clone())?;
        let elapsed = current.at.saturating_sub(previous.at);
        let pairs = [
            (current.counters.received, previous.counters.received),
            (current.counters.decoded, previous.counters.decoded),
            (current.counters.presented, previous.counters.presented),
            (
                current.counters.packets_received,
                previous.counters.packets_received,
            ),
            (current.submitted, previous.submitted),
        ];
        if elapsed == 0
            || elapsed > 1_500
            || current.peer != previous.peer
            || current.video != previous.video
            || current.renderer != previous.renderer
            || current.renderer_kind != previous.renderer_kind
            || pairs
                .iter()
                .any(|(n, p)| n.zip(*p).is_some_and(|(n, p)| n < p))
        {
            self.since = None;
            self.domain = None;
            return None;
        }
        let delta = |n: Option<u64>, p: Option<u64>| n.zip(p).map(|(n, p)| n.saturating_sub(p));
        let inbound = delta(
            current.counters.packets_received,
            previous.counters.packets_received,
        )
        .is_some_and(|n| n >= 12)
            || delta(current.counters.received, previous.counters.received).is_some_and(|n| n > 0);
        let decoded = delta(current.counters.decoded, previous.counters.decoded);
        let (final_delta, age) = match current.renderer_kind.as_deref() {
            Some("video") => (
                delta(current.counters.presented, previous.counters.presented),
                sample.video_present_age_ms,
            ),
            Some("webgl2" | "webgl2_sr") if current.renderer.is_some() => (
                delta(current.submitted, previous.submitted),
                sample.renderer_submit_age_ms,
            ),
            _ => (None, None),
        };
        let domain = if inbound && final_delta == Some(0) {
            match decoded {
                Some(0) => Some("decode"),
                Some(_) => Some("presentation"),
                None => None,
            }
        } else {
            None
        };
        let Some(domain) = domain else {
            self.since = None;
            self.domain = None;
            return None;
        };
        if self.domain != Some(domain) {
            self.since = Some(previous.at);
            self.domain = Some(domain);
        }
        let age = age.filter(|n| n.is_finite() && *n >= 0.0)? as u64;
        let coverage = current.at.saturating_sub(self.since?);
        if coverage < 1_000
            || age < runtime_stall_threshold(sample.connected_elapsed_ms, sample.frame_interval_ms)
        {
            return None;
        }
        Some(RuntimeLocalStallDecision {
            domain: domain.into(),
            action: if domain == "decode" {
                "keyframeRequest"
            } else {
                "playbackRebuild"
            }
            .into(),
            age_ms: age,
            coverage_ms: coverage,
            eligible: false,
            reason: "confirmedLocalStall".into(),
        })
    }
}
