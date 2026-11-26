//! 显示观察按 render 作用域独立记账，网络与解码资格继续由原监督窗口承担。
use super::{RuntimeSupervisionSample, SOFT_WINDOW_MS, STABLE_WINDOW_MS};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RuntimeDisplayObservation {
    pub source: String,
    pub active: bool,
    pub surface_ready: bool,
    pub render_epoch: u64,
    pub surface_revision: u64,
    pub sampled_at_ms: u64,
    pub submitted: u64,
    pub gpu_completed: u64,
    pub displayed: u64,
    pub last_submit_at_ms: Option<f64>,
    pub last_completed_at_ms: Option<f64>,
    pub last_displayed_at_ms: Option<f64>,
    pub first_displayed_at_ms: Option<f64>,
    pub last_frame_id: Option<u64>,
    pub drawable_acquire_age_ms: Option<f64>,
    pub inflight_age_ms: Option<f64>,
    pub frame_interval_ms: Option<f64>,
    pub submit_to_present_p95_ms: Option<f64>,
    pub submit_to_present_p99_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDisplayRecoveryAction {
    pub id: u64,
    pub render_epoch: u64,
    pub surface_revision: u64,
    pub observed_at_ms: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDisplayRecoveryCompletion {
    pub id: u64,
    pub result: String,
    pub completed_at_ms: u64,
    pub render_epoch: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDisplayRecoveryDecision {
    pub eligible: bool,
    pub active: bool,
    pub exhausted: bool,
    pub reason: String,
    pub age_ms: u64,
    pub coverage_ms: u64,
    pub threshold_ms: u64,
    pub action: Option<RuntimeDisplayRecoveryAction>,
    pub effect: Option<String>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    display: RuntimeDisplayObservation,
    received: u64,
    decoded: u64,
}

#[derive(Debug, Clone)]
struct Pending {
    action: RuntimeDisplayRecoveryAction,
    frame_id: Option<u64>,
    acknowledged_at: Option<u64>,
    acknowledged_epoch: Option<u64>,
    healthy_since: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DisplayRecoveryState {
    previous: Option<Snapshot>,
    pressure_since: Option<u64>,
    healthy_since: Option<u64>,
    normal_interval_ms: Option<f64>,
    budget_used: bool,
    next_action_id: u64,
    pending: Option<Pending>,
}

impl DisplayRecoveryState {
    pub fn observe(
        &mut self,
        sample: &RuntimeSupervisionSample,
        fresh: bool,
        duplicate: bool,
    ) -> Option<RuntimeDisplayRecoveryDecision> {
        let display = sample.display_observation.as_ref()?;
        let valid = fresh
            && sample.visible
            && !sample.is_closing
            && sample.connection_state == "connected"
            && sample.control_channel_state == "open"
            && display.active
            && display.surface_ready
            && matches!(display.source.as_str(), "drawablePresented" | "gpuProxy")
            && display.sampled_at_ms <= sample.now_ms
            && sample.now_ms - display.sampled_at_ms <= 250;
        if !valid {
            self.previous = None;
            self.pressure_since = None;
            self.healthy_since = None;
            let cancelled = self.pending.take().is_some();
            return Some(RuntimeDisplayRecoveryDecision {
                reason: "displayObservationInactive".into(),
                effect: cancelled.then(|| "superseded".into()),
                ..Default::default()
            });
        }
        if duplicate {
            return Some(RuntimeDisplayRecoveryDecision {
                active: self.pending.is_some(),
                exhausted: self.budget_used,
                reason: "duplicateDisplaySample".into(),
                ..Default::default()
            });
        }
        let at = display.sampled_at_ms;
        let finite_time = |value: Option<f64>| {
            value.filter(|n| n.is_finite() && *n >= 0.0 && *n <= sample.now_ms as f64)
        };
        let age =
            finite_time(display.last_displayed_at_ms).map(|n| (sample.now_ms as f64 - n) as u64);
        let threshold = (8.0 * self.normal_interval_ms.unwrap_or(16.7)).max(500.0) as u64;
        let mut decision = RuntimeDisplayRecoveryDecision {
            threshold_ms: threshold,
            age_ms: age.unwrap_or(0),
            exhausted: self.budget_used,
            reason: "displayHealthy".into(),
            ..Default::default()
        };
        if let Some(completion) = sample.display_recovery_completion.as_ref() {
            if let Some(pending) = self
                .pending
                .as_mut()
                .filter(|p| p.action.id == completion.id)
            {
                if completion.result == "acknowledged" && pending.acknowledged_at.is_none() {
                    pending.acknowledged_at = Some(completion.completed_at_ms);
                    pending.acknowledged_epoch = Some(completion.render_epoch);
                } else if matches!(completion.result.as_str(), "failed" | "timedOut") {
                    self.pending = None;
                    decision.reason = "displayRecoveryFailed".into();
                    decision.effect = Some("degraded".into());
                    return Some(decision);
                } else if completion.result == "superseded" {
                    self.pending = None;
                    decision.reason = "displayRecoverySuperseded".into();
                    decision.effect = Some("superseded".into());
                    return Some(decision);
                }
            }
        }
        let counters = sample.counters.as_ref()?;
        let current = Snapshot {
            display: display.clone(),
            received: counters.received?,
            decoded: counters.decoded?,
        };
        let previous = self.previous.replace(current.clone());
        let continuous = previous.as_ref().is_some_and(|old| {
            at > old.display.sampled_at_ms
                && at - old.display.sampled_at_ms <= 1_500
                && current.received >= old.received
                && current.decoded >= old.decoded
        });
        let supplied = continuous
            && previous.as_ref().is_some_and(|old| {
                current.received > old.received && current.decoded > old.decoded
            });
        let same_surface = previous.as_ref().is_some_and(|old| {
            display.render_epoch == old.display.render_epoch
                && display.surface_revision == old.display.surface_revision
                && display.source == old.display.source
                && display.displayed >= old.display.displayed
        });
        let progressed = same_surface
            && previous
                .as_ref()
                .is_some_and(|old| display.displayed > old.display.displayed);
        let healthy = supplied && progressed && age.is_some_and(|age| age < 250);
        if healthy {
            self.pressure_since = None;
            self.healthy_since = Some(self.healthy_since.unwrap_or(at));
            self.normal_interval_ms = display
                .frame_interval_ms
                .filter(|n| n.is_finite() && (1.0..=1_000.0).contains(n))
                .or(self.normal_interval_ms);
            if self.pending.is_none()
                && at.saturating_sub(self.healthy_since.unwrap_or(at)) >= STABLE_WINDOW_MS
                && sample.long.received_coverage_ms >= STABLE_WINDOW_MS - 1
                && sample.long.decode_coverage_ms >= STABLE_WINDOW_MS - 1
            {
                self.budget_used = false;
                decision.exhausted = false;
            }
        } else {
            self.healthy_since = None;
        }
        if let Some(pending) = self.pending.as_mut() {
            decision.active = true;
            decision.reason = "displayRecoveryObserving".into();
            if let Some(ack) = pending.acknowledged_at {
                let fresh_frame = pending.acknowledged_epoch == Some(display.render_epoch)
                    && display.last_frame_id.is_some()
                    && display.last_frame_id != pending.frame_id
                    && finite_time(display.last_displayed_at_ms).is_some_and(|n| n >= ack as f64);
                if supplied && fresh_frame && age.is_some_and(|n| n < 250) {
                    pending.healthy_since = Some(pending.healthy_since.unwrap_or_else(|| {
                        finite_time(display.first_displayed_at_ms)
                            .filter(|n| *n >= ack as f64)
                            .map(|n| n as u64)
                            .unwrap_or(at)
                    }));
                } else {
                    pending.healthy_since = None;
                }
                let improved = pending
                    .healthy_since
                    .is_some_and(|since| at.saturating_sub(since) >= SOFT_WINDOW_MS - 1);
                if improved || at.saturating_sub(ack) >= SOFT_WINDOW_MS + 1_000 {
                    self.pending = None;
                    decision.active = false;
                    decision.reason = if improved {
                        "displayRecovered"
                    } else {
                        "displayRecoveryFailed"
                    }
                    .into();
                    decision.effect = Some(if improved { "improved" } else { "degraded" }.into());
                }
            } else if sample.now_ms.saturating_sub(pending.action.observed_at_ms) >= 1_500 {
                self.pending = None;
                decision.active = false;
                decision.reason = "displayRecoveryAckTimedOut".into();
                decision.effect = Some("degraded".into());
            }
            return Some(decision);
        }
        let stalled = supplied
            && same_surface
            && !progressed
            && display.displayed > 0
            && age.is_some_and(|age| age >= threshold)
            && sample.now_ms >= sample.warmup_until_ms;
        if stalled {
            self.pressure_since = Some(self.pressure_since.unwrap_or_else(|| {
                previous
                    .as_ref()
                    .map(|p| p.display.sampled_at_ms)
                    .unwrap_or(at)
            }));
        } else {
            self.pressure_since = None;
        }
        decision.coverage_ms = self
            .pressure_since
            .map(|since| at.saturating_sub(since))
            .unwrap_or(0);
        decision.reason = if stalled {
            "displayStalled"
        } else {
            "displayHealthy"
        }
        .into();
        if stalled
            && decision.coverage_ms >= threshold
            && !self.budget_used
            && sample.probe.is_none()
        {
            self.next_action_id += 1;
            let stage = if display
                .drawable_acquire_age_ms
                .is_some_and(|n| n >= threshold as f64)
            {
                "drawableAcquireStalled"
            } else if display
                .inflight_age_ms
                .is_some_and(|n| n >= threshold as f64)
            {
                "gpuCompletionStalled"
            } else {
                "drawablePresentationStalled"
            };
            let action = RuntimeDisplayRecoveryAction {
                id: self.next_action_id,
                render_epoch: display.render_epoch,
                surface_revision: display.surface_revision,
                observed_at_ms: sample.now_ms,
                reason: stage.into(),
            };
            self.pending = Some(Pending {
                action: action.clone(),
                frame_id: display.last_frame_id,
                acknowledged_at: None,
                acknowledged_epoch: None,
                healthy_since: None,
            });
            self.budget_used = true;
            decision.eligible = true;
            decision.active = true;
            decision.exhausted = true;
            decision.action = Some(action);
        }
        Some(decision)
    }
}

#[cfg(test)]
#[path = "display.test.rs"]
mod tests;
