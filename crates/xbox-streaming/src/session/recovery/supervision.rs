//! Browser 与原生宿主共同消费的媒体监督事实和纯判定。
use super::observation::{RuntimeMediaCounters, RuntimeWindowAccumulator};
use super::quality::{RuntimeQualityCompletion, RuntimeQualityDecision, RuntimeQualityState};
use serde::{Deserialize, Serialize};

#[path = "local_stall.rs"]
mod local_stall;
use local_stall::LocalStallState;
pub use local_stall::RuntimeLocalStallDecision;

#[path = "display.rs"]
mod display;
use display::DisplayRecoveryState;
pub use display::{
    RuntimeDisplayObservation, RuntimeDisplayRecoveryAction, RuntimeDisplayRecoveryCompletion,
    RuntimeDisplayRecoveryDecision,
};

pub const FIRST_FRAME_GUARD_MS: u64 = 4_000;
pub const SUPERVISION_INTERVAL_MS: u64 = 1_000;
pub const SOFT_WINDOW_MS: u64 = 3_000;
pub const STABLE_WINDOW_MS: u64 = 15_000;
pub const RECOVERY_ARBITER_WINDOW_MS: u64 = 6_000;
pub const CONNECTION_RECOVERY_TIMEOUT_MS: u64 = 120_000;
pub const CONNECTION_RECOVERY_ACTION_BUDGET: u8 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RuntimeObservationWindow {
    pub received_coverage_ms: u64,
    pub decode_coverage_ms: u64,
    pub video_coverage_ms: u64,
    pub inbound_fps_avg: Option<f64>,
    pub decode_fps_avg: Option<f64>,
    pub video_present_fps_avg: Option<f64>,
    pub loss_coverage_ms: u64,
    pub loss_avg: Option<f64>,
    pub rtt_coverage_ms: u64,
    pub rtt_ms_avg: Option<f64>,
    pub jitter_coverage_ms: u64,
    pub jitter_ms_avg: Option<f64>,
    pub jitter_buffer_coverage_ms: u64,
    pub jitter_buffer_ms_first: Option<f64>,
    pub jitter_buffer_ms_last: Option<f64>,
    pub repair_packets_per_second_avg: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RuntimeSupervisionSample {
    pub now_ms: u64,
    pub sampled_at_ms: Option<u64>,
    pub sample_id: Option<String>,
    pub observation_fresh: bool,
    pub visible: bool,
    pub is_closing: bool,
    pub connection_state: String,
    pub control_channel_state: String,
    pub connected_elapsed_ms: u64,
    pub inactivity_elapsed_ms: u64,
    pub frame_interval_ms: Option<f64>,
    pub inbound_bytes_total: Option<u64>,
    pub inbound_video_packet_count_total: Option<u64>,
    pub inbound_video_fps: Option<f64>,
    pub decode_fps: Option<f64>,
    pub video_present_age_ms: Option<f64>,
    pub packet_age_ms: Option<f64>,
    pub peer_generation: u64,
    pub video_generation: u64,
    pub warmup_until_ms: u64,
    pub previous_action_at_ms: Option<u64>,
    pub previous_action_kind: Option<String>,
    pub previous_action_effect: Option<String>,
    pub has_presented_media: bool,
    pub local_counters: Option<RuntimeMediaCounters>,
    pub renderer_generation: Option<u64>,
    pub renderer_kind: Option<String>,
    pub renderer_submitted_total: Option<u64>,
    pub renderer_submit_age_ms: Option<f64>,
    pub renderer_present_fps: Option<f64>,
    pub renderer_present_coverage_ms: Option<u64>,
    pub eligible_since_ms: u64,
    pub short: RuntimeObservationWindow,
    pub long: RuntimeObservationWindow,
    pub probe: Option<RuntimeRecoveryProbe>,
    pub rtt_baseline_ms: Option<f64>,
    pub counters: Option<RuntimeMediaCounters>,
    pub rtt_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub jitter_buffer_ms: Option<f64>,
    pub path_id: Option<String>,
    pub quality_level: u8,
    pub quality_supported: bool,
    pub quality_completion: Option<RuntimeQualityCompletion>,
    pub request_connection_recovery: bool,
    pub connection_recovery_followup: bool,
    pub connection_recovery_action_completed: bool,
    pub connection_recovery_fact_key: Option<String>,
    pub display_observation: Option<RuntimeDisplayObservation>,
    pub display_recovery_completion: Option<RuntimeDisplayRecoveryCompletion>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeSupervisionState {
    sample_id: Option<String>,
    sampled_at_ms: Option<u64>,
    peer_generation: Option<u64>,
    pressure_since_ms: Option<u64>,
    healthy_since_ms: Option<u64>,
    eligible_since_ms: u64,
    inbound_bytes: Option<u64>,
    inbound_packets: Option<u64>,
    previous_action_at_ms: Option<u64>,
    stall_evidence_observed: bool,
    windows: RuntimeWindowAccumulator,
    local_stall: LocalStallState,
    display: DisplayRecoveryState,
    quality: RuntimeQualityState,
    connection_started_ms: Option<u64>,
    connection_last_action_ms: Option<u64>,
    connection_last_fact_key: Option<String>,
    connection_last_reason: Option<String>,
    connection_observing: bool,
    connection_actions: u8,
    connection_stable_since_ms: Option<u64>,
    connection_last_frame_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSupervisionDecision {
    pub fresh: bool,
    pub duplicate: bool,
    pub stall_threshold_ms: u64,
    pub has_inbound_progress: bool,
    pub decode_recovery_eligible: bool,
    pub decode_pressure_sustained_ms: u64,
    pub healthy_episode_complete: bool,
    pub media_connection_eligible: bool,
    pub network_pressure: String,
    pub probe_effect: Option<RuntimeRecoveryEffect>,
    pub local_stall: Option<RuntimeLocalStallDecision>,
    pub display_recovery_eligible: bool,
    pub display_recovery: Option<RuntimeDisplayRecoveryDecision>,
    pub short: RuntimeObservationWindow,
    pub quality: RuntimeQualityDecision,
    pub connection_recovery_allowed: bool,
    pub connection_recovery_exhausted: bool,
    pub connection_recovery_active: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RuntimeRecoveryBaseline {
    pub decode_fps: Option<f64>,
    pub video_fps: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoveryProbe {
    pub action: String,
    pub started_at_ms: u64,
    pub observe_ms: u64,
    pub required_observe_ms: u64,
    pub sample_id: Option<String>,
    pub video_generation: Option<u64>,
    pub peer_generation: Option<u64>,
    pub renderer_generation: Option<u64>,
    pub recovery_scope: Option<String>,
    pub baseline: RuntimeRecoveryBaseline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoveryEffect {
    pub result: String,
    pub reason: String,
    pub score: Option<f64>,
}

pub fn runtime_stall_threshold(connected_elapsed_ms: u64, frame_interval_ms: Option<f64>) -> u64 {
    if connected_elapsed_ms < STABLE_WINDOW_MS {
        return 8_000;
    }
    frame_interval_ms
        .filter(|n| n.is_finite() && *n > 0.0)
        .map(|n| (n * 12.0).clamp(1_500.0, 7_000.0) as u64)
        .unwrap_or(5_000)
}

pub fn classify_runtime_network_pressure(
    window: &RuntimeObservationWindow,
    baseline: Option<f64>,
) -> &'static str {
    assess_runtime_network_pressure(window, baseline).pressure
}

pub(super) struct RuntimeNetworkAssessment {
    pub pressure: &'static str,
    pub reason: &'static str,
}

pub(super) fn assess_runtime_network_pressure(
    window: &RuntimeObservationWindow,
    baseline: Option<f64>,
) -> RuntimeNetworkAssessment {
    let finite = |value: Option<f64>| value.filter(|n| n.is_finite() && *n >= 0.0);
    let loss = finite(window.loss_avg).filter(|_| window.loss_coverage_ms >= SOFT_WINDOW_MS - 1);
    let baseline = finite(baseline);
    let rtt_growth = baseline
        .zip(finite(window.rtt_ms_avg))
        .filter(|_| window.rtt_coverage_ms >= SOFT_WINDOW_MS - 1)
        .map(|(a, b)| b - a);
    let delay_pressure =
        rtt_growth.is_some_and(|growth| growth > 50.0_f64.max(baseline.unwrap_or(0.0) * 0.25));
    let jitter = window.jitter_coverage_ms >= SOFT_WINDOW_MS - 1
        && finite(window.jitter_ms_avg).is_some_and(|n| n >= 35.0);
    let buffer_growth = window.jitter_buffer_coverage_ms >= SOFT_WINDOW_MS - 1
        && finite(window.jitter_buffer_ms_last)
            .zip(finite(window.jitter_buffer_ms_first))
            .is_some_and(|(last, first)| last - first >= 100.0);
    if loss.is_some_and(|loss| {
        loss >= 0.08
            || (loss >= 0.03
                && (delay_pressure
                    || jitter
                    || (buffer_growth
                        && window.repair_packets_per_second_avg.unwrap_or(0.0) > 0.0)))
    }) {
        RuntimeNetworkAssessment {
            pressure: "congested",
            reason: "transportLoss",
        }
    } else if delay_pressure {
        RuntimeNetworkAssessment {
            pressure: if buffer_growth {
                "congested"
            } else {
                "warning"
            },
            reason: "delayAccumulation",
        }
    } else if loss.is_some_and(|loss| loss >= 0.03) {
        RuntimeNetworkAssessment {
            pressure: "warning",
            reason: "transportLoss",
        }
    } else if loss.is_some() {
        RuntimeNetworkAssessment {
            pressure: "stable",
            reason: "pathStable",
        }
    } else {
        RuntimeNetworkAssessment {
            pressure: "unknown",
            reason: "networkEvidenceIncomplete",
        }
    }
}

pub fn decide_runtime_supervision(
    sample: &RuntimeSupervisionSample,
    state: &mut RuntimeSupervisionState,
) -> RuntimeSupervisionDecision {
    let generation_current = state
        .peer_generation
        .is_none_or(|peer| sample.peer_generation >= peer);
    if !generation_current {
        return RuntimeSupervisionDecision {
            fresh: false,
            duplicate: false,
            stall_threshold_ms: runtime_stall_threshold(
                sample.connected_elapsed_ms,
                sample.frame_interval_ms,
            ),
            has_inbound_progress: false,
            decode_recovery_eligible: false,
            decode_pressure_sustained_ms: 0,
            healthy_episode_complete: false,
            media_connection_eligible: false,
            network_pressure: "unknown".into(),
            probe_effect: None,
            local_stall: None,
            display_recovery_eligible: false,
            display_recovery: None,
            short: RuntimeObservationWindow::default(),
            quality: RuntimeQualityDecision::default(),
            connection_recovery_allowed: false,
            connection_recovery_exhausted: false,
            connection_recovery_active: false,
        };
    }
    if generation_current && state.peer_generation != Some(sample.peer_generation) {
        let previous = std::mem::take(state);
        state.connection_started_ms = previous.connection_started_ms;
        state.connection_last_action_ms = previous.connection_last_action_ms;
        state.connection_actions = previous.connection_actions;
        state.connection_last_fact_key = previous.connection_last_fact_key;
        state.connection_last_reason = previous.connection_last_reason;
        state.connection_observing = previous.connection_observing;
        state.peer_generation = Some(sample.peer_generation);
    }
    let at = sample.sampled_at_ms.unwrap_or(0);
    state.eligible_since_ms = state.eligible_since_ms.max(sample.eligible_since_ms);
    let fresh = generation_current
        && sample.observation_fresh
        && sample
            .sample_id
            .as_ref()
            .is_some_and(|s| !s.is_empty() && s.len() <= 256)
        && sample.sampled_at_ms.is_some()
        && sample.now_ms >= at
        && sample.now_ms - at <= SUPERVISION_INTERVAL_MS
        && sample.visible
        && !sample.is_closing;
    let mut projected = sample.clone();
    if fresh {
        if let Some(counters) = &sample.counters {
            state.windows.observe(
                at,
                counters,
                sample.rtt_ms,
                sample.jitter_ms,
                sample.jitter_buffer_ms,
            );
            projected.short = state.windows.window(at, SOFT_WINDOW_MS);
            projected.long = state.windows.window(at, STABLE_WINDOW_MS);
            projected.inbound_video_fps = projected.short.inbound_fps_avg;
            projected.decode_fps = projected.short.decode_fps_avg;
        }
    } else if sample.counters.is_some() {
        state.windows = RuntimeWindowAccumulator::default();
    }
    let sample = &projected;
    let duplicate = fresh
        && (sample.sample_id == state.sample_id
            || state.sampled_at_ms.is_some_and(|last| at <= last));
    let continuity_lost = !fresh
        || state
            .sampled_at_ms
            .is_some_and(|last| at.saturating_sub(last) > 1_500);
    if continuity_lost {
        state.pressure_since_ms = None;
        state.healthy_since_ms = None;
        if !fresh {
            state.eligible_since_ms = sample.now_ms;
        }
    }
    if state.previous_action_at_ms != sample.previous_action_at_ms {
        state.pressure_since_ms = None;
        state.healthy_since_ms = None;
        state.previous_action_at_ms = sample.previous_action_at_ms;
    }
    let short = &sample.short;
    let covered = at.saturating_sub(SOFT_WINDOW_MS) >= state.eligible_since_ms
        && short.received_coverage_ms >= SOFT_WINDOW_MS - 1
        && short.decode_coverage_ms >= SOFT_WINDOW_MS - 1;
    let received = short.inbound_fps_avg.unwrap_or(0.0);
    if !duplicate {
        let pressure = fresh
            && covered
            && received > 0.0
            && short.decode_fps_avg.is_some_and(|n| n < received * 0.66);
        state.pressure_since_ms = if pressure {
            Some(state.pressure_since_ms.unwrap_or(at))
        } else {
            None
        };
        let healthy = fresh
            && covered
            && short.video_coverage_ms >= SOFT_WINDOW_MS - 1
            && received > 0.0
            && short.decode_fps_avg.is_some_and(|n| n >= received * 0.8)
            && short.video_present_fps_avg.unwrap_or(0.0) > 0.0
            && sample.video_present_age_ms.is_some_and(|n| n < 500.0);
        state.healthy_since_ms = if healthy {
            Some(state.healthy_since_ms.unwrap_or(at))
        } else {
            None
        };
    }
    let action_cooled = sample
        .previous_action_at_ms
        .is_none_or(|last| at.saturating_sub(last) >= STABLE_WINDOW_MS);
    let probe_effect = sample
        .probe
        .as_ref()
        .and_then(|probe| evaluate_runtime_recovery_probe(probe, sample, fresh && !duplicate));
    let mut local_stall = state.local_stall.observe(sample, fresh, duplicate);
    if let Some(local) = &mut local_stall {
        let effect = probe_effect
            .as_ref()
            .map(|effect| effect.result.as_str())
            .or(sample.previous_action_effect.as_deref());
        let upgrade = sample.previous_action_kind.as_deref() == Some("keyframeRequest")
            && matches!(effect, Some("neutral" | "degraded"))
            && sample
                .previous_action_at_ms
                .is_some_and(|last| at.saturating_sub(last) >= SOFT_WINDOW_MS);
        local.eligible =
            (action_cooled || upgrade) && (sample.probe.is_none() || probe_effect.is_some());
        if upgrade {
            local.action = "playbackRebuild".into();
            local.reason = "ineffectiveKeyframeUpgrade".into();
        } else if !local.eligible {
            local.reason = "localActionCooldown".into();
        }
    }
    let decode_recovery_eligible = fresh
        && !duplicate
        && action_cooled
        && sample.now_ms >= sample.warmup_until_ms
        && state
            .pressure_since_ms
            .is_some_and(|since| at.saturating_sub(since) >= SOFT_WINDOW_MS);
    let healthy_episode_complete = fresh
        && !duplicate
        && action_cooled
        && state
            .healthy_since_ms
            .is_some_and(|since| at.saturating_sub(since) >= STABLE_WINDOW_MS)
        && sample.long.received_coverage_ms >= STABLE_WINDOW_MS - 1
        && sample.long.decode_coverage_ms >= STABLE_WINDOW_MS - 1
        && sample.long.video_coverage_ms >= STABLE_WINDOW_MS - 1;
    let has_inbound_progress = local_stall.is_some()
        || (fresh
            && !duplicate
            && state.stall_evidence_observed
            && (sample
                .inbound_bytes_total
                .zip(state.inbound_bytes)
                .is_some_and(|(n, p)| n.saturating_sub(p) > 16 * 1024)
                || sample
                    .inbound_video_packet_count_total
                    .zip(state.inbound_packets)
                    .is_some_and(|(n, p)| n.saturating_sub(p) >= 12)
                || sample.inbound_video_fps.unwrap_or(0.0) >= 1.0
                || sample.decode_fps.unwrap_or(0.0) >= 1.0));
    if fresh && !duplicate {
        state.sample_id = sample.sample_id.clone();
        state.sampled_at_ms = Some(at);
        if sample.inactivity_elapsed_ms
            >= runtime_stall_threshold(sample.connected_elapsed_ms, sample.frame_interval_ms)
        {
            state.stall_evidence_observed = true;
            state.inbound_bytes = sample.inbound_bytes_total;
            state.inbound_packets = sample.inbound_video_packet_count_total;
        }
    }
    let supplied = has_inbound_progress
        || sample.inbound_video_fps.unwrap_or(0.0) >= 1.0
        || sample.decode_fps.unwrap_or(0.0) >= 1.0
        || sample.packet_age_ms.is_some_and(|n| n < 1_500.0);
    let media_connection_eligible = fresh
        && !duplicate
        && match sample.connection_state.as_str() {
            "failed" | "closed" => true,
            "connected" => {
                matches!(sample.control_channel_state.as_str(), "closed" | "closing") || !supplied
            }
            _ => false,
        };
    let quality = state
        .quality
        .observe(sample, fresh, duplicate, state.eligible_since_ms);
    if sample.connection_recovery_action_completed && state.connection_started_ms.is_some() {
        state.connection_observing = true;
    }
    if state.connection_observing
        && fresh
        && !duplicate
        && sample.video_present_age_ms.is_some_and(|age| age < 500.0)
    {
        if state
            .connection_last_frame_ms
            .is_none_or(|last| at.saturating_sub(last) > 1_500)
        {
            state.connection_stable_since_ms = Some(at);
        }
        state.connection_last_frame_ms = Some(at);
        if state
            .connection_stable_since_ms
            .is_some_and(|since| at.saturating_sub(since) >= 5_000)
        {
            state.connection_started_ms = None;
            state.connection_last_action_ms = None;
            state.connection_actions = 0;
            state.connection_last_fact_key = None;
            state.connection_last_reason = None;
            state.connection_observing = false;
        }
    }
    let expired = state
        .connection_started_ms
        .is_some_and(|start| sample.now_ms.saturating_sub(start) >= CONNECTION_RECOVERY_TIMEOUT_MS);
    let fact_key = sample
        .connection_recovery_fact_key
        .as_deref()
        .unwrap_or("mediaStalled");
    let reason = super::decide_runtime_recovery(
        if fact_key == "directPathExhausted" {
            super::RuntimeFact::TransportConnectionState("failed")
        } else if fact_key.starts_with("transportConnectionState:") {
            super::RuntimeFact::TransportConnectionState(&sample.connection_state)
        } else {
            super::RuntimeFact::MediaStalled
        },
        sample.is_closing,
    )
    .map(|reason| reason.as_str());
    let within_window = state
        .connection_last_action_ms
        .is_some_and(|last| sample.now_ms.saturating_sub(last) < RECOVERY_ARBITER_WINDOW_MS);
    let same_fact_or_reason = state.connection_last_fact_key.as_deref() == Some(fact_key)
        || reason.is_some() && state.connection_last_reason.as_deref() == reason;
    let connection_recovery_allowed = sample.request_connection_recovery
        && !sample.is_closing
        && sample.visible
        && !expired
        && !state.connection_observing
        && reason.is_some()
        && state.connection_actions < CONNECTION_RECOVERY_ACTION_BUDGET
        && (sample.connection_recovery_followup || !within_window || !same_fact_or_reason);
    if connection_recovery_allowed {
        state.connection_started_ms.get_or_insert(sample.now_ms);
        state.connection_last_action_ms = Some(sample.now_ms);
        state.connection_last_fact_key = Some(fact_key.into());
        state.connection_last_reason = reason.map(str::to_owned);
        state.connection_actions += 1;
        state.connection_stable_since_ms = None;
        state.connection_last_frame_ms = None;
    }
    let display_recovery = state.display.observe(sample, fresh, duplicate);
    RuntimeSupervisionDecision {
        fresh,
        duplicate,
        stall_threshold_ms: runtime_stall_threshold(
            sample.connected_elapsed_ms,
            sample.frame_interval_ms,
        ),
        has_inbound_progress,
        decode_recovery_eligible,
        decode_pressure_sustained_ms: state
            .pressure_since_ms
            .map(|since| at.saturating_sub(since))
            .unwrap_or(0),
        healthy_episode_complete,
        media_connection_eligible,
        network_pressure: if fresh
            && !duplicate
            && at.saturating_sub(SOFT_WINDOW_MS) >= state.eligible_since_ms
        {
            classify_runtime_network_pressure(short, quality.rtt_baseline_ms)
        } else {
            "unknown"
        }
        .into(),
        probe_effect,
        local_stall,
        display_recovery_eligible: display_recovery
            .as_ref()
            .is_some_and(|decision| decision.eligible),
        display_recovery,
        short: sample.short.clone(),
        quality,
        connection_recovery_allowed,
        connection_recovery_active: state.connection_started_ms.is_some(),
        connection_recovery_exhausted: expired
            || (sample.request_connection_recovery
                && state.connection_actions >= CONNECTION_RECOVERY_ACTION_BUDGET
                && !state.connection_observing
                && !connection_recovery_allowed),
    }
}

pub fn evaluate_runtime_recovery_probe(
    probe: &RuntimeRecoveryProbe,
    sample: &RuntimeSupervisionSample,
    fresh: bool,
) -> Option<RuntimeRecoveryEffect> {
    let elapsed = sample.now_ms.saturating_sub(probe.started_at_ms);
    if elapsed < probe.observe_ms {
        return None;
    }
    let unknown = |reason: &str| RuntimeRecoveryEffect {
        result: "unknown".into(),
        reason: reason.into(),
        score: None,
    };
    if probe.required_observe_ms > STABLE_WINDOW_MS {
        return Some(unknown("pathResponseExceedsDeadline"));
    }
    let peer_valid = if probe.action == "reconnect" {
        probe
            .peer_generation
            .is_some_and(|p| sample.peer_generation != p)
    } else {
        probe.peer_generation == Some(sample.peer_generation)
    };
    let renderer_scope = probe.recovery_scope.as_deref() == Some("renderer");
    let playback_valid = probe.action != "playbackRebuild"
        || (renderer_scope
            && probe
                .renderer_generation
                .is_some_and(|p| sample.renderer_generation.is_some_and(|n| n != p)))
        || (!renderer_scope
            && probe
                .video_generation
                .is_some_and(|p| sample.video_generation != p));
    let presentation_coverage = if renderer_scope {
        sample.renderer_present_coverage_ms.unwrap_or(0)
    } else {
        sample.short.video_coverage_ms
    };
    let presentation_fps = if renderer_scope {
        sample.renderer_present_fps
    } else {
        sample.short.video_present_fps_avg
    };
    let presentation_age = if renderer_scope {
        sample.renderer_submit_age_ms
    } else {
        sample.video_present_age_ms
    };
    let covered = fresh
        && sample.sample_id != probe.sample_id
        && sample
            .sampled_at_ms
            .is_some_and(|at| at.saturating_sub(SOFT_WINDOW_MS) >= probe.started_at_ms)
        && sample.short.decode_coverage_ms >= SOFT_WINDOW_MS - 1
        && presentation_coverage >= SOFT_WINDOW_MS - 1
        && peer_valid
        && playback_valid;
    if !covered {
        return (elapsed >= STABLE_WINDOW_MS).then(|| unknown("postActionEvidenceIncomplete"));
    }
    let decode = sample.short.decode_fps_avg;
    let score = decode.zip(probe.baseline.decode_fps).map(|(n, p)| n - p);
    let margin = 1.0_f64.max(probe.baseline.decode_fps.unwrap_or(0.0) * 0.1);
    let new_frames = decode.unwrap_or(0.0) > 0.0
        && presentation_fps.unwrap_or(0.0) > 0.0
        && presentation_age.is_some_and(|n| n < 500.0);
    let improved = new_frames
        && (matches!(probe.action.as_str(), "reconnect" | "playbackRebuild")
            || score.is_some_and(|n| n > margin)
            || probe.baseline.video_fps.is_some_and(|n| n < 1.0)
                && presentation_fps.unwrap_or(0.0) >= 1.0);
    Some(RuntimeRecoveryEffect {
        result: if improved {
            "improved"
        } else if score.is_some_and(|n| n < -margin) {
            "degraded"
        } else {
            "neutral"
        }
        .into(),
        reason: if improved {
            "freshVideoRecovered"
        } else {
            "localSupplyUnchanged"
        }
        .into(),
        score,
    })
}

#[cfg(test)]
#[path = "supervision.test.rs"]
mod tests;
