use super::supervision::{
    classify_runtime_network_pressure, RuntimeObservationWindow, RuntimeSupervisionSample,
    SOFT_WINDOW_MS, STABLE_WINDOW_MS,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeQualityAction {
    pub id: u64,
    pub level: u8,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeQualityCompletion {
    pub id: u64,
    pub result: String,
    pub previous_level: u8,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeQualityDecision {
    pub state: String,
    pub reason: String,
    pub rtt_baseline_ms: Option<f64>,
    pub rtt_delta_ms: Option<f64>,
    pub effect: Option<String>,
    pub action: Option<RuntimeQualityAction>,
}

impl Default for RuntimeQualityDecision {
    fn default() -> Self {
        Self {
            state: "unknown".into(),
            reason: "observationUnavailable".into(),
            rtt_baseline_ms: None,
            rtt_delta_ms: None,
            effect: None,
            action: None,
        }
    }
}

#[derive(Debug, Clone)]
struct Flight {
    action: RuntimeQualityAction,
    baseline: RuntimeObservationWindow,
    invalidated: bool,
}

#[derive(Debug, Clone)]
struct Probe {
    action: RuntimeQualityAction,
    previous_level: u8,
    at: u64,
    required_ms: f64,
    baseline: RuntimeObservationWindow,
}

/// 与 PC 基准相同的网络资格、观察、回退和单动作冷却。
#[derive(Debug, Clone, Default)]
pub(super) struct RuntimeQualityState {
    path: Option<String>,
    pressure_since: Option<u64>,
    healthy_since: Option<u64>,
    last_sample_at: Option<u64>,
    last_action_at: Option<u64>,
    eligible_since: u64,
    rtt_baseline: Option<f64>,
    sequence: u64,
    flight: Option<Flight>,
    probe: Option<Probe>,
    restore_level: Option<u8>,
    suspended: bool,
    effect: Option<String>,
    decision: RuntimeQualityDecision,
}

impl RuntimeQualityState {
    pub(super) fn observe(
        &mut self,
        sample: &RuntimeSupervisionSample,
        fresh: bool,
        duplicate: bool,
        eligible_since: u64,
    ) -> RuntimeQualityDecision {
        if duplicate {
            let mut decision = self.decision.clone();
            decision.action = None;
            return decision;
        }
        if self.path != sample.path_id {
            let sequence = self.sequence;
            *self = Self::default();
            self.sequence = sequence;
            self.path = sample.path_id.clone();
        }
        if eligible_since > self.eligible_since {
            self.eligible_since = eligible_since;
            self.pressure_since = None;
            self.healthy_since = None;
            if let Some(flight) = &mut self.flight {
                flight.invalidated = true;
            }
            if let Some(probe) = self.probe.take() {
                self.effect = Some("unknown".into());
                self.restore_level = Some(probe.previous_level);
                self.suspended = true;
            }
        }
        if let Some(completion) = &sample.quality_completion {
            if self
                .flight
                .as_ref()
                .is_some_and(|flight| flight.action.id == completion.id)
            {
                let flight = self.flight.take().unwrap();
                self.last_action_at = Some(completion.completed_at_ms.min(sample.now_ms));
                if completion.result == "applied" && !flight.invalidated {
                    if flight.action.reason != "ineffectiveRestore" {
                        self.probe = Some(Probe {
                            action: flight.action,
                            previous_level: completion.previous_level,
                            at: self.last_action_at.unwrap(),
                            required_ms: (sample.rtt_ms.or(self.rtt_baseline).unwrap_or(0.0) * 2.0)
                                .max(SOFT_WINDOW_MS as f64),
                            baseline: flight.baseline,
                        });
                    }
                } else {
                    self.effect = Some("unknown".into());
                    self.suspended = true;
                    if completion.result == "applied" {
                        self.restore_level = Some(completion.previous_level);
                    }
                }
            }
        }
        let at = sample.sampled_at_ms.unwrap_or(sample.now_ms);
        let new_sample = fresh && self.last_sample_at.is_none_or(|last| at > last);
        if self
            .last_sample_at
            .is_some_and(|last| at.saturating_sub(last) > 1_500)
            || !fresh
        {
            self.pressure_since = None;
            self.healthy_since = None;
        }
        let pressure = if new_sample && at.saturating_sub(SOFT_WINDOW_MS) >= self.eligible_since {
            classify_runtime_network_pressure(&sample.short, self.rtt_baseline)
        } else {
            "unknown"
        };
        if new_sample {
            self.last_sample_at = Some(at);
        }
        if pressure == "stable" {
            self.healthy_since.get_or_insert(sample.now_ms);
            self.pressure_since = None;
            if self.rtt_baseline.is_none() && sample.short.rtt_coverage_ms >= SOFT_WINDOW_MS - 1 {
                self.rtt_baseline = sample.short.rtt_ms_avg;
            }
            if self
                .healthy_since
                .is_some_and(|since| sample.now_ms.saturating_sub(since) >= STABLE_WINDOW_MS)
                && sample.long.rtt_coverage_ms >= STABLE_WINDOW_MS - 1
            {
                self.rtt_baseline = sample.long.rtt_ms_avg;
            }
        } else {
            self.healthy_since = None;
            if pressure == "congested" {
                self.pressure_since.get_or_insert(sample.now_ms);
            } else {
                self.pressure_since = None;
            }
        }
        let mut effect_evaluated = false;
        if let Some(probe) = &self.probe {
            let elapsed = sample.now_ms.saturating_sub(probe.at);
            if elapsed as f64 >= probe.required_ms.min(STABLE_WINDOW_MS as f64) {
                let post = &sample.short;
                let covered = new_sample
                    && at.saturating_sub(SOFT_WINDOW_MS) >= probe.at
                    && post.loss_coverage_ms >= SOFT_WINDOW_MS - 1;
                if covered || elapsed >= STABLE_WINDOW_MS {
                    let supply_held = post.received_coverage_ms >= SOFT_WINDOW_MS - 1
                        && post
                            .inbound_fps_avg
                            .zip(probe.baseline.inbound_fps_avg)
                            .is_some_and(|(n, p)| n >= p * 0.95);
                    let effect = if probe.required_ms > STABLE_WINDOW_MS as f64
                        || !covered
                        || !supply_held
                        || post.loss_avg.is_none()
                        || probe.baseline.loss_avg.is_none()
                    {
                        "unknown"
                    } else if probe.action.reason == "healthyRestore" {
                        if pressure == "stable" {
                            "improved"
                        } else {
                            "degraded"
                        }
                    } else if post.loss_avg.unwrap() < probe.baseline.loss_avg.unwrap() * 0.5 {
                        "improved"
                    } else if post.loss_avg.unwrap() > probe.baseline.loss_avg.unwrap() * 1.2 {
                        "degraded"
                    } else {
                        "neutral"
                    };
                    self.effect = Some(effect.into());
                    effect_evaluated = true;
                    if effect != "improved" {
                        self.suspended = true;
                        self.restore_level = Some(probe.previous_level);
                    }
                    self.probe = None;
                }
            }
        }
        let mut action = None;
        if new_sample
            && sample.quality_supported
            && self.flight.is_none()
            && self.probe.is_none()
            && sample.now_ms >= sample.warmup_until_ms
            && self
                .last_action_at
                .is_none_or(|at| sample.now_ms.saturating_sub(at) >= STABLE_WINDOW_MS)
        {
            let request = if let Some(level) = self.restore_level.take() {
                (level != sample.quality_level).then_some((level, "ineffectiveRestore"))
            } else if !self.suspended
                && pressure == "stable"
                && sample.quality_level > 0
                && self
                    .healthy_since
                    .is_some_and(|at| sample.now_ms.saturating_sub(at) >= STABLE_WINDOW_MS)
                && sample.long.loss_coverage_ms >= STABLE_WINDOW_MS - 1
            {
                Some((0, "healthyRestore"))
            } else if !self.suspended
                && pressure == "congested"
                && sample.quality_level < 2
                && self
                    .pressure_since
                    .is_some_and(|at| sample.now_ms.saturating_sub(at) >= SOFT_WINDOW_MS)
            {
                Some((sample.quality_level + 1, "sustainedCongestion"))
            } else {
                None
            };
            if let Some((level, reason)) = request {
                self.sequence += 1;
                let reserved = RuntimeQualityAction {
                    id: self.sequence,
                    level,
                    reason: reason.into(),
                };
                self.flight = Some(Flight {
                    action: reserved.clone(),
                    baseline: sample.short.clone(),
                    invalidated: false,
                });
                action = Some(reserved);
            }
        }
        let decision = RuntimeQualityDecision {
            state: if pressure == "stable" && sample.quality_level > 0 {
                "recovering"
            } else {
                pressure
            }
            .into(),
            reason: if !sample.quality_supported && pressure == "congested" {
                "remoteQualityCapabilityUnknown".into()
            } else if effect_evaluated {
                self.effect
                    .as_ref()
                    .map(|effect| format!("qualityEffect:{effect}"))
                    .unwrap_or_else(|| {
                        if pressure == "unknown" {
                            "networkEvidenceIncomplete"
                        } else if pressure == "stable" {
                            "pathStable"
                        } else {
                            "transportLoss"
                        }
                        .into()
                    })
            } else {
                if pressure == "unknown" {
                    "networkEvidenceIncomplete"
                } else if pressure == "stable" {
                    "pathStable"
                } else {
                    "transportLoss"
                }
                .into()
            },
            rtt_baseline_ms: self.rtt_baseline,
            rtt_delta_ms: sample
                .short
                .rtt_ms_avg
                .zip(self.rtt_baseline)
                .map(|(n, p)| n - p),
            effect: self.effect.clone(),
            action,
        };
        self.decision = decision.clone();
        decision
    }
}
