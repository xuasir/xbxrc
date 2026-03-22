use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::transport::rtc::recovery::coordinator::RecoveryCouplingMode;
use crate::transport::rtc::recovery::coordinator::RecoveryCouplingState;
use crate::transport::rtc::recovery::policy::{ScenarioPolicyProfileKind, ScenarioPolicyResolver};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::{XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig};

pub(crate) struct BweDecision {
    pub(crate) target_kbps: u32,
    pub(crate) reason: String,
}

pub(crate) struct TwccGccInput<'a> {
    pub(crate) observation: &'a XbxEngineVideoTwccObservation,
    pub(crate) rtt_ms: Option<f64>,
}

pub(crate) fn resolve_transport_policy_profile_kind(
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
) -> ScenarioPolicyProfileKind {
    ScenarioPolicyResolver::resolve_kind(session_target_type, transport_path)
}

pub(crate) fn classify_scenario_bitrate_band(
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
    actual_video_bitrate_kbps: Option<f64>,
) -> Option<&'static str> {
    ScenarioPolicyResolver::classify_bitrate_band(
        session_target_type,
        transport_path,
        actual_video_bitrate_kbps,
    )
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
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
    session_phase: SessionPhase,
    recovery_coupling: Option<RecoveryCouplingState>,
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
    let transport_profile = ScenarioPolicyResolver::resolve_transport_bwe_profile(
        config,
        session_target_type,
        transport_path,
        session_phase,
    );

    let (next_kbps, reason) = match config.bwe_mode.as_str() {
        "twcc-gcc" => resolve_twcc_gcc_target(
            config,
            current_kbps,
            actual_headroom_kbps,
            transport_profile,
            recovery_coupling,
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
                    recovery_coupling,
                    Some(twcc),
                    hybrid_ramp_cooldown_ticks,
                )
            } else {
                if is_severe_rtt(transport_profile, rtt_ms) {
                    *hybrid_ramp_cooldown_ticks = transport_profile.severe_cooldown_ticks;
                    (
                        ((current_kbps as f64) * (config.remb_ramp_down_factor as f64 / 1000.0))
                            .round()
                            .max(floor_kbps as f64)
                            .min(ceiling_kbps as f64) as u32,
                        "hybrid-severe-rtt-backoff".to_string(),
                    )
                } else if is_high_rtt(transport_profile, rtt_ms) {
                    *hybrid_ramp_cooldown_ticks =
                        transport_profile.congestion_cooldown_ticks.max(1);
                    (
                        current_kbps.clamp(floor_kbps, ceiling_kbps),
                        "hybrid-high-rtt-hold".to_string(),
                    )
                } else {
                    let severe_loss = loss_ratio >= 0.08;
                    let sustained_loss = loss_ratio >= 0.01;
                    let mild_loss = loss_ratio >= 0.005;
                    let bitrate_overrun = actual_kbps > (current_kbps as f64 * 1.1);

                    if severe_loss || bitrate_overrun {
                        *hybrid_ramp_cooldown_ticks = 12;
                        (
                            ((current_kbps as f64) * (config.remb_ramp_down_factor as f64 / 1000.0))
                                .round()
                                .max(floor_kbps as f64) as u32,
                            if severe_loss {
                                "hybrid-severe-loss-backoff".to_string()
                            } else {
                                "hybrid-bitrate-overrun-backoff".to_string()
                            },
                        )
                    } else if sustained_loss {
                        *hybrid_ramp_cooldown_ticks = 10;
                        (
                            current_kbps.min(actual_headroom_kbps).max(floor_kbps),
                            "hybrid-sustained-loss-cap".to_string(),
                        )
                    } else if mild_loss {
                        *hybrid_ramp_cooldown_ticks = 6;
                        (
                            current_kbps
                                .min(
                                    actual_headroom_kbps
                                        .saturating_add(config.remb_ramp_up_step_kbps),
                                )
                                .max(floor_kbps),
                            "hybrid-mild-loss-hold".to_string(),
                        )
                    } else if *hybrid_ramp_cooldown_ticks > 0 {
                        *hybrid_ramp_cooldown_ticks = hybrid_ramp_cooldown_ticks.saturating_sub(1);
                        (current_kbps, "hybrid-ramp-cooldown".to_string())
                    } else {
                        let desired_kbps = observed_kbps.min(ceiling_kbps);
                        (
                            current_kbps
                                .saturating_add(config.remb_ramp_up_step_kbps)
                                .min(desired_kbps)
                                .max(floor_kbps),
                            if observed_remb_kbps.is_some() {
                                "hybrid-ramp-up-observed".to_string()
                            } else {
                                "hybrid-ramp-up-ceiling".to_string()
                            },
                        )
                    }
                }
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
    recovery_coupling: Option<RecoveryCouplingState>,
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
    let effective_peak_ceiling_kbps = if bounded_gaming_profile
        && recovery_coupling
            .as_ref()
            .is_some_and(|coupling| !coupling.allow_peak_range)
    {
        operating_ceiling_kbps.max(preferred_gaming_floor_kbps)
    } else {
        peak_ceiling_kbps
    };
    let ramp_up_step_kbps = profile.ramp_up_step_kbps;
    let fast_ramp_up_step_kbps = profile.fast_ramp_up_step_kbps;
    let Some(twcc_input) = twcc_input else {
        return (
            current_kbps
                .max(preferred_gaming_floor_kbps)
                .min(effective_peak_ceiling_kbps.min(ceiling_kbps)),
            if recovery_coupling
                .as_ref()
                .is_some_and(|coupling| coupling.prefer_hold)
            {
                profile.reason("coupled-await-feedback-hold")
            } else {
                profile.reason("await-feedback")
            },
        );
    };

    let twcc = twcc_input.observation;
    let rtt_ms = twcc_input.rtt_ms;
    let stable_feedback = twcc.feedback_interval_ms.unwrap_or(0.0)
        <= profile.stable_feedback_interval_ms
        && twcc.observed_packet_count >= profile.stable_feedback_min_packets
        && twcc.covered_sequence_span >= twcc.observed_packet_count;
    let raw_receive_bitrate_kbps = twcc
        .receive_bitrate_kbps
        .unwrap_or(actual_headroom_kbps as f64)
        .max(1.0);
    let raw_receive_headroom_kbps =
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

    if !stable_feedback {
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

    if bounded_gaming_profile
        && recovery_coupling
            .as_ref()
            .is_some_and(|coupling| coupling.suppress_ramp_up || coupling.prefer_hold)
    {
        let coupling = recovery_coupling.expect("checked above");
        let (hold_floor_kbps, hold_ceiling_kbps, reason_suffix) = match coupling.mode {
            RecoveryCouplingMode::StartupLowQuality => (
                preferred_gaming_floor_kbps,
                operating_ceiling_kbps.max(preferred_gaming_floor_kbps),
                "recovery-coupled-startup-hold",
            ),
            RecoveryCouplingMode::WaitingKeyframe => (
                preferred_gaming_floor_kbps,
                operating_ceiling_kbps.max(preferred_gaming_floor_kbps),
                "recovery-coupled-wait-keyframe-hold",
            ),
            RecoveryCouplingMode::RecoveringReferenceChain => (
                desired_kbps
                    .max(actual_headroom_kbps)
                    .max(operating_ceiling_kbps.max(preferred_gaming_floor_kbps))
                    .clamp(
                        preferred_gaming_floor_kbps,
                        effective_peak_ceiling_kbps.min(ceiling_kbps),
                    ),
                effective_peak_ceiling_kbps.min(ceiling_kbps),
                "recovery-coupled-reference-hold",
            ),
            RecoveryCouplingMode::Stalled => (
                preferred_gaming_floor_kbps,
                operating_ceiling_kbps.max(preferred_gaming_floor_kbps),
                "recovery-coupled-stall-hold",
            ),
            RecoveryCouplingMode::ThinStream => (
                preferred_gaming_floor_kbps,
                operating_ceiling_kbps.max(preferred_gaming_floor_kbps),
                "recovery-coupled-thin-stream-hold",
            ),
            RecoveryCouplingMode::Healthy => (
                desired_kbps
                    .max(actual_headroom_kbps)
                    .max(operating_ceiling_kbps.max(preferred_gaming_floor_kbps))
                    .clamp(
                        preferred_gaming_floor_kbps,
                        effective_peak_ceiling_kbps.min(ceiling_kbps),
                    ),
                effective_peak_ceiling_kbps.min(ceiling_kbps),
                "recovery-coupled-hold",
            ),
        };

        // 启动低质态如果已经明显拥塞，就不要继续被 25k floor 顶住。
        // 这里直接按真实 TWCC headroom 回退，先把发送压力压下来。
        if matches!(coupling.mode, RecoveryCouplingMode::StartupLowQuality)
            && matches!(profile.kind, ScenarioPolicyProfileKind::CloudGaming)
            && (twcc.packet_loss_ratio >= profile.severe_loss_threshold
                || twcc.delivery_ratio <= profile.severe_delivery_threshold)
        {
            *ramp_cooldown_ticks = profile.congestion_cooldown_ticks.max(2);
            return (
                raw_receive_headroom_kbps.min(desired_kbps.max(1)),
                profile.reason("recovery-coupled-startup-backoff"),
            );
        }

        // Cloud 场景下，reference chain 恢复不应该一直“顶着”不降码率。
        // 一旦 TWCC 已经进入明显拥塞，就先退回到保守 backoff，
        // 否则会把短时抖动放大成持续 burst loss。
        if matches!(
            coupling.mode,
            RecoveryCouplingMode::RecoveringReferenceChain
        ) && (twcc.packet_loss_ratio >= profile.congestion_loss_threshold
            || twcc.delivery_ratio <= profile.congestion_delivery_threshold)
        {
            *ramp_cooldown_ticks = profile.congestion_cooldown_ticks.max(1);
            let recovery_backoff_floor_kbps = profile.preferred_floor(floor_kbps);
            let backoff_kbps = ((current_kbps as f64)
                * (config.remb_ramp_down_factor as f64 / 1000.0))
                .round()
                .max(recovery_backoff_floor_kbps as f64) as u32;
            return (
                if bounded_gaming_profile {
                    backoff_kbps
                        .min(desired_kbps.max(recovery_backoff_floor_kbps))
                        .max(recovery_backoff_floor_kbps)
                } else {
                    backoff_kbps.min(receive_headroom_kbps.max(floor_kbps))
                },
                profile.reason("recovery-coupled-reference-backoff"),
            );
        }

        return (
            current_kbps
                .max(hold_floor_kbps)
                .clamp(preferred_gaming_floor_kbps, hold_ceiling_kbps),
            profile.reason(reason_suffix),
        );
    }

    // RTT 是 transport 侧的直接拥塞信号，优先阻止“loss 还没起但排队已明显增加”的错误 ramp-up。
    if is_severe_rtt(profile, rtt_ms) {
        *ramp_cooldown_ticks = profile.severe_cooldown_ticks;
        let backoff_kbps = ((current_kbps as f64) * (config.remb_ramp_down_factor as f64 / 1000.0))
            .round()
            .max(floor_kbps as f64) as u32;
        return (
            if bounded_gaming_profile {
                backoff_kbps
                    .min(desired_kbps.max(preferred_gaming_floor_kbps))
                    .max(preferred_gaming_floor_kbps)
            } else {
                backoff_kbps.min(receive_headroom_kbps.max(floor_kbps))
            },
            profile.reason("severe-rtt-backoff"),
        );
    }

    if is_high_rtt(profile, rtt_ms) {
        *ramp_cooldown_ticks = profile.congestion_cooldown_ticks.max(1);
        return (
            if bounded_gaming_profile {
                current_kbps
                    .min(effective_peak_ceiling_kbps.min(ceiling_kbps))
                    .max(preferred_gaming_floor_kbps)
            } else {
                current_kbps.min(receive_headroom_kbps.max(floor_kbps))
            },
            profile.reason("high-rtt-hold"),
        );
    }

    if twcc.packet_loss_ratio >= profile.severe_loss_threshold
        || twcc.delivery_ratio <= profile.severe_delivery_threshold
    {
        *ramp_cooldown_ticks = profile.severe_cooldown_ticks;
        let backoff_kbps = ((current_kbps as f64) * (config.remb_ramp_down_factor as f64 / 1000.0))
            .round()
            .max(floor_kbps as f64) as u32;
        return (
            if bounded_gaming_profile {
                backoff_kbps
                    .min(desired_kbps.max(preferred_gaming_floor_kbps))
                    .max(preferred_gaming_floor_kbps)
            } else {
                backoff_kbps.min(receive_headroom_kbps.max(floor_kbps))
            },
            profile.reason("severe-backoff"),
        );
    }

    if twcc.packet_loss_ratio >= profile.congestion_loss_threshold
        || twcc.delivery_ratio <= profile.congestion_delivery_threshold
    {
        *ramp_cooldown_ticks = profile.congestion_cooldown_ticks;
        let cap_kbps = desired_kbps
            .min(effective_peak_ceiling_kbps.min(ceiling_kbps))
            .max(preferred_gaming_floor_kbps);
        return (
            if bounded_gaming_profile {
                current_kbps.min(cap_kbps)
            } else {
                current_kbps.min(receive_headroom_kbps.max(floor_kbps))
            },
            profile.reason("congestion-cap"),
        );
    }

    if twcc.packet_loss_ratio >= profile.mild_loss_threshold
        || twcc.delivery_ratio <= profile.mild_delivery_threshold
    {
        *ramp_cooldown_ticks = profile.mild_cooldown_ticks;
        return (
            if bounded_gaming_profile {
                current_kbps
                    .min(effective_peak_ceiling_kbps.min(ceiling_kbps))
                    .max(preferred_gaming_floor_kbps)
            } else {
                current_kbps
                    .min(receive_headroom_kbps.saturating_add(ramp_up_step_kbps))
                    .max(floor_kbps)
            },
            profile.reason("mild-hold"),
        );
    }

    if *ramp_cooldown_ticks > 0 {
        *ramp_cooldown_ticks = ramp_cooldown_ticks.saturating_sub(1);
        return (
            if bounded_gaming_profile {
                current_kbps
                    .saturating_add(profile.cooldown_recovery_step_kbps.unwrap_or(0))
                    .min(desired_kbps)
                    .min(effective_peak_ceiling_kbps.min(ceiling_kbps))
                    .max(preferred_gaming_floor_kbps)
            } else {
                current_kbps.max(preferred_gaming_floor_kbps)
            },
            profile.reason("ramp-cooldown"),
        );
    }

    let desired_kbps = if bounded_gaming_profile {
        desired_kbps
            .max(actual_headroom_kbps.min(effective_peak_ceiling_kbps.min(ceiling_kbps)))
            .clamp(
                preferred_gaming_floor_kbps,
                effective_peak_ceiling_kbps.min(ceiling_kbps),
            )
    } else {
        receive_headroom_kbps
            .max(actual_headroom_kbps)
            .max(preferred_gaming_floor_kbps)
            .clamp(floor_kbps, ceiling_kbps)
    };
    (
        current_kbps
            .saturating_add(
                if bounded_gaming_profile && desired_kbps >= peak_enter_kbps {
                    fast_ramp_up_step_kbps
                } else {
                    ramp_up_step_kbps
                },
            )
            .min(desired_kbps)
            .max(floor_kbps),
        profile.reason("ramp-up"),
    )
}

fn is_high_rtt(
    profile: crate::transport::rtc::recovery::policy::TransportBweScenarioProfile,
    rtt_ms: Option<f64>,
) -> bool {
    match (profile.high_rtt_ms_threshold, rtt_ms) {
        (Some(threshold_ms), Some(value_ms)) => value_ms >= threshold_ms,
        _ => false,
    }
}

fn is_severe_rtt(
    profile: crate::transport::rtc::recovery::policy::TransportBweScenarioProfile,
    rtt_ms: Option<f64>,
) -> bool {
    match (profile.severe_rtt_ms_threshold, rtt_ms) {
        (Some(threshold_ms), Some(value_ms)) => value_ms >= threshold_ms,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_scenario_bitrate_band, resolve_target_remb_kbps,
        resolve_transport_policy_profile_kind, resolve_twcc_gcc_target, RecoveryCouplingState,
        SessionPhase,
    };
    use crate::transport::rtc::recovery::coordinator::RecoveryCouplingMode;
    use crate::transport::rtc::recovery::policy::ScenarioPolicyResolver;
    use crate::{XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig};
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    #[test]
    fn profile_kind_prioritizes_session_target_type_over_transport_path() {
        assert_eq!(
            resolve_transport_policy_profile_kind(
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)")
            )
            .as_str(),
            "cloudGaming"
        );
        assert_eq!(
            resolve_transport_policy_profile_kind(
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)")
            )
            .as_str(),
            "homeLanGaming"
        );
        assert_eq!(
            resolve_transport_policy_profile_kind(
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Relay")
            )
            .as_str(),
            "relayGaming"
        );
    }

    #[test]
    fn scenario_bitrate_band_matches_home_and_cloud_operating_ranges() {
        assert_eq!(
            classify_scenario_bitrate_band(
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)"),
                Some(16_000.0),
            ),
            Some("operatingRange")
        );
        assert_eq!(
            classify_scenario_bitrate_band(
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                Some(22_000.0),
            ),
            Some("operatingRange")
        );
        assert_eq!(
            classify_scenario_bitrate_band(
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                Some(28_000.0),
            ),
            Some("peakRange")
        );
        assert_eq!(
            classify_scenario_bitrate_band(
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Relay"),
                Some(16_000.0),
            ),
            None
        );
    }

    #[test]
    fn direct_recovery_coupling_holds_bwe_and_caps_peak() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 1,
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 120,
            covered_sequence_span: 120,
            observed_packet_count: 120,
            observed_byte_count: 120_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(100.0),
            receive_bitrate_kbps: Some(30_000.0),
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 1.0,
        };
        let coupling = RecoveryCouplingState {
            mode: RecoveryCouplingMode::RecoveringReferenceChain,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            18_000,
            20_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)"),
                SessionPhase::Recovering,
            ),
            Some(coupling),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: None,
            }),
            &mut cooldown,
        );
        assert_eq!(target, 23_000);
        assert_eq!(reason, "twcc-gcc-direct-recovery-coupled-reference-hold");
    }

    #[test]
    fn cloud_recovery_reference_chain_backs_off_under_congestion() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 3,
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 80,
            covered_sequence_span: 80,
            observed_packet_count: 32,
            observed_byte_count: 32_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(100.0),
            receive_bitrate_kbps: Some(8_000.0),
            delivery_ratio: 0.40,
            packet_loss_ratio: 0.60,
            observed_at_ms: 3.0,
        };
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.remb_floor_kbps = 25_000;
        config.remb_ceiling_kbps = 50_000;
        let coupling = RecoveryCouplingState {
            mode: RecoveryCouplingMode::RecoveringReferenceChain,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &config,
            25_000,
            25_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &config,
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                SessionPhase::Recovering,
            ),
            Some(coupling),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: None,
            }),
            &mut cooldown,
        );

        assert_eq!(target, 21_250);
        assert_eq!(reason, "twcc-gcc-cloud-recovery-coupled-reference-backoff");
        assert_eq!(cooldown, 2);
    }

    #[test]
    fn cloud_wait_keyframe_coupling_holds_at_cloud_operating_floor() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 1,
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 120,
            covered_sequence_span: 120,
            observed_packet_count: 120,
            observed_byte_count: 120_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(100.0),
            receive_bitrate_kbps: Some(24_000.0),
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 1.0,
        };
        let coupling = RecoveryCouplingState {
            mode: RecoveryCouplingMode::WaitingKeyframe,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            12_000,
            14_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                SessionPhase::Startup,
            ),
            Some(coupling),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: None,
            }),
            &mut cooldown,
        );
        assert_eq!(target, 20_000);
        assert_eq!(reason, "twcc-gcc-cloud-recovery-coupled-wait-keyframe-hold");
    }

    #[test]
    fn cloud_startup_low_quality_backs_off_under_congestion() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 4,
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 120,
            covered_sequence_span: 120,
            observed_packet_count: 24,
            observed_byte_count: 24_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(100.0),
            receive_bitrate_kbps: Some(5_000.0),
            delivery_ratio: 0.08,
            packet_loss_ratio: 0.92,
            observed_at_ms: 4.0,
        };
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.remb_floor_kbps = 25_000;
        config.remb_ceiling_kbps = 50_000;
        let coupling = RecoveryCouplingState {
            mode: RecoveryCouplingMode::StartupLowQuality,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &config,
            25_000,
            25_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &config,
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                SessionPhase::Startup,
            ),
            Some(coupling),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: None,
            }),
            &mut cooldown,
        );

        assert_eq!(target, 5_500);
        assert_eq!(reason, "twcc-gcc-cloud-recovery-coupled-startup-backoff");
        assert_eq!(cooldown, 2);
    }

    #[test]
    fn direct_high_rtt_holds_ramp_up_even_when_twcc_is_clean() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 2,
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 160,
            covered_sequence_span: 160,
            observed_packet_count: 160,
            observed_byte_count: 180_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(100.0),
            receive_bitrate_kbps: Some(25_000.0),
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            20_000,
            21_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)"),
                SessionPhase::Steady,
            ),
            None,
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(90.0),
            }),
            &mut cooldown,
        );

        assert_eq!(target, 20_000);
        assert_eq!(reason, "twcc-gcc-direct-high-rtt-hold");
        assert_eq!(cooldown, 1);
    }

    #[test]
    fn fixed_mode_resolve_target_remb_returns_forced_value() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "fixed".to_string();
        config.forced_remb_kbps = Some(22_000);
        let mut last_sent = 18_000;
        let mut cooldown = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            12_000.0,
            0.0,
            None,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            None,
            &mut last_sent,
            &mut cooldown,
        );

        assert_eq!(decision.target_kbps, 22_000);
        assert_eq!(decision.reason, "fixed-remb");
        assert_eq!(last_sent, 22_000);
    }
}
