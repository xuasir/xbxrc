#[path = "policy/hybrid_rules.rs"]
mod hybrid_rules;
#[path = "policy/twcc_rules.rs"]
mod twcc_rules;

use xbxengine_protocol::XbxEngineTargetTypeDto;

use self::hybrid_rules::{resolve_hybrid_target, HybridRuleContext};
use self::twcc_rules::{
    resolve_cooldown_or_ramp, resolve_loss_rule, resolve_rtt_rule, TwccRuleContext,
};
use crate::transport::rtc::recovery::policy::{ScenarioPolicyProfileKind, ScenarioPolicyResolver};
use crate::transport::rtc::recovery::remote_profile_runtime::resolve_runtime_profile_kind;
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::{
    XbxEngineTwccObservationQuality, XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig,
};

pub(crate) struct BweDecision {
    pub(crate) target_kbps: u32,
    pub(crate) reason: String,
}

pub(crate) struct TwccGccInput<'a> {
    pub(crate) observation: &'a XbxEngineVideoTwccObservation,
    pub(crate) rtt_ms: Option<f64>,
}

pub(crate) fn resolve_transport_policy_profile_kind(
    baseline_remote_profile: Option<&str>,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
) -> ScenarioPolicyProfileKind {
    resolve_runtime_profile_kind(baseline_remote_profile, session_target_type, transport_path)
}

pub(crate) fn classify_scenario_bitrate_band(
    baseline_remote_profile: Option<&str>,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
    actual_video_bitrate_kbps: Option<f64>,
) -> Option<&'static str> {
    let bitrate_kbps = actual_video_bitrate_kbps?;
    let profile_kind = resolve_transport_policy_profile_kind(
        baseline_remote_profile,
        session_target_type,
        transport_path,
    );
    ScenarioPolicyResolver::classify_bitrate_band_by_kind(profile_kind, bitrate_kbps)
}

/**
 * Policy 层只负责根据实时输入生成 target 与 reason。
 * 这里统一消费策略层产出的 scenario profile，执行层不再自行判 direct/relay。
 */
pub(crate) fn resolve_target_remb_kbps(
    config: &XbxEngineWebRtcRuntimeConfig,
    observed_remb_kbps: Option<u32>,
    actual_kbps: f64,
    loss_ratio: f64,
    rtt_ms: Option<f64>,
    baseline_remote_profile: Option<&str>,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
    session_phase: SessionPhase,
    twcc_observation: Option<&XbxEngineVideoTwccObservation>,
    last_sent_remb_kbps: &mut u32,
    hybrid_ramp_cooldown_ticks: &mut u8,
) -> BweDecision {
    let floor_kbps = config.remb_floor_kbps.max(1);
    let ceiling_kbps = config.remb_ceiling_kbps.max(floor_kbps);
    let forced_kbps = config
        .forced_remb_kbps
        .unwrap_or(ceiling_kbps)
        .clamp(floor_kbps, ceiling_kbps);
    let observed_kbps = observed_remb_kbps
        .unwrap_or(forced_kbps)
        .clamp(floor_kbps, ceiling_kbps);
    let current_kbps = (*last_sent_remb_kbps).clamp(floor_kbps, ceiling_kbps);
    let actual_headroom_kbps =
        ((actual_kbps * 1.25).round() as u32).clamp(floor_kbps, ceiling_kbps);
    let twcc_input = twcc_observation.map(|observation| TwccGccInput {
        observation,
        rtt_ms,
    });
    let profile_kind = resolve_transport_policy_profile_kind(
        baseline_remote_profile,
        session_target_type,
        transport_path,
    );
    let transport_profile = ScenarioPolicyResolver::resolve_transport_bwe_profile_by_kind(
        config,
        profile_kind,
        session_phase,
    );

    let (next_kbps, reason) = match config.bwe_mode.as_str() {
        "twcc-gcc" => resolve_twcc_gcc_target(
            config,
            current_kbps,
            actual_headroom_kbps,
            transport_profile,
            twcc_input.as_ref(),
            hybrid_ramp_cooldown_ticks,
        ),
        "observed-remb" => (observed_kbps, "observed-remb".to_string()),
        "hybrid" => {
            if let Some(twcc) = twcc_input.as_ref() {
                resolve_twcc_gcc_target(
                    config,
                    current_kbps,
                    actual_headroom_kbps,
                    transport_profile,
                    Some(twcc),
                    hybrid_ramp_cooldown_ticks,
                )
            } else {
                resolve_hybrid_target(
                    HybridRuleContext {
                        config,
                        transport_profile,
                        current_kbps,
                        observed_kbps,
                        actual_kbps,
                        actual_headroom_kbps,
                        loss_ratio,
                        rtt_ms,
                        observed_remb_present: observed_remb_kbps.is_some(),
                        floor_kbps,
                        ceiling_kbps,
                    },
                    hybrid_ramp_cooldown_ticks,
                )
            }
        }
        _ => (forced_kbps, "fixed-remb".to_string()),
    };

    let clamped_kbps = next_kbps.clamp(floor_kbps, ceiling_kbps);
    *last_sent_remb_kbps = clamped_kbps;
    BweDecision {
        target_kbps: clamped_kbps,
        reason,
    }
}

/**
 * TWCC/GCC 执行层只消费策略层给出的带宽区间和阈值，不自行做场景分类。
 */
pub(crate) fn resolve_twcc_gcc_target(
    config: &XbxEngineWebRtcRuntimeConfig,
    current_kbps: u32,
    actual_headroom_kbps: u32,
    profile: crate::transport::rtc::recovery::policy::TransportBweScenarioProfile,
    twcc_input: Option<&TwccGccInput<'_>>,
    ramp_cooldown_ticks: &mut u8,
) -> (u32, String) {
    let floor_kbps = config.remb_floor_kbps.max(1);
    let ceiling_kbps = config.remb_ceiling_kbps.max(floor_kbps);
    let bounded_gaming_profile = profile.kind != ScenarioPolicyProfileKind::RelayGaming;
    let preferred_gaming_floor_kbps = floor_kbps.max(profile.preferred_floor(floor_kbps));
    let operating_ceiling_kbps = profile
        .operating_ceiling_kbps
        .unwrap_or(preferred_gaming_floor_kbps);
    let peak_enter_kbps = profile.peak_enter_kbps.unwrap_or(ceiling_kbps);
    let peak_ceiling_kbps = profile.peak_ceiling(ceiling_kbps);
    let effective_peak_ceiling_kbps = peak_ceiling_kbps;
    let ramp_up_step_kbps = profile.ramp_up_step_kbps;
    let fast_ramp_up_step_kbps = profile.fast_ramp_up_step_kbps;
    let Some(twcc_input) = twcc_input else {
        return (
            current_kbps
                .max(preferred_gaming_floor_kbps)
                .min(effective_peak_ceiling_kbps.min(ceiling_kbps)),
            profile.reason("await-feedback"),
        );
    };

    let twcc = twcc_input.observation;
    let rtt_ms = twcc_input.rtt_ms;
    let quality = twcc.classify_quality(
        config.video_pipeline.feedback_interval_ms as f64,
        profile.stable_feedback_interval_ms,
        profile.stable_feedback_min_packets,
    );
    let stable_feedback = matches!(
        quality,
        XbxEngineTwccObservationQuality::Stable
            | XbxEngineTwccObservationQuality::RemoteObserved
            | XbxEngineTwccObservationQuality::Delayed
            | XbxEngineTwccObservationQuality::BootstrapSparse
    );
    let should_bypass_unstable_hold = quality == XbxEngineTwccObservationQuality::BootstrapSparse;
    let raw_receive_bitrate_kbps = twcc
        .receive_bitrate_kbps
        .unwrap_or(actual_headroom_kbps as f64)
        .max(1.0);
    let _raw_receive_headroom_kbps =
        (raw_receive_bitrate_kbps * profile.receive_headroom_factor).round() as u32;
    let receive_bitrate_kbps =
        raw_receive_bitrate_kbps.clamp(floor_kbps as f64, ceiling_kbps as f64) as u32;
    let receive_headroom_kbps = ((receive_bitrate_kbps as f64) * profile.receive_headroom_factor)
        .round()
        .clamp(floor_kbps as f64, ceiling_kbps as f64) as u32;
    let desired_kbps = if bounded_gaming_profile {
        let bounded_receive = receive_headroom_kbps
            .max(preferred_gaming_floor_kbps)
            .min(effective_peak_ceiling_kbps.min(ceiling_kbps));
        if bounded_receive >= peak_enter_kbps {
            bounded_receive
        } else {
            bounded_receive
                .min(operating_ceiling_kbps.max(preferred_gaming_floor_kbps))
                .max(preferred_gaming_floor_kbps)
        }
    } else {
        receive_headroom_kbps.max(floor_kbps)
    };

    if !stable_feedback && !should_bypass_unstable_hold {
        return (
            if bounded_gaming_profile {
                current_kbps.clamp(
                    preferred_gaming_floor_kbps,
                    effective_peak_ceiling_kbps.min(ceiling_kbps),
                )
            } else {
                current_kbps.min(receive_headroom_kbps.max(floor_kbps))
            },
            profile.reason("unstable-hold"),
        );
    }

    let rule_context = TwccRuleContext {
        config,
        twcc,
        profile,
        bounded_gaming_profile,
        current_kbps,
        actual_headroom_kbps,
        desired_kbps,
        receive_headroom_kbps,
        floor_kbps,
        ceiling_kbps,
        preferred_gaming_floor_kbps,
        effective_peak_ceiling_kbps,
        peak_enter_kbps,
        ramp_up_step_kbps,
        fast_ramp_up_step_kbps,
    };

    // RTT 是 transport 侧的直接拥塞信号，优先阻止“loss 还没起但排队已明显增加”的错误 ramp-up。
    if let Some(decision) = resolve_rtt_rule(rule_context, rtt_ms, ramp_cooldown_ticks) {
        return decision;
    }

    if let Some(decision) = resolve_loss_rule(rule_context, ramp_cooldown_ticks) {
        return decision;
    }

    resolve_cooldown_or_ramp(rule_context, ramp_cooldown_ticks)
}

#[cfg(test)]
#[path = "policy.test.rs"]
mod tests;
