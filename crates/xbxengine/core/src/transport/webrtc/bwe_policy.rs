use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::transport::webrtc::policy::{ScenarioPolicyProfileKind, ScenarioPolicyResolver};
use crate::transport::webrtc::recovery_coordinator::RecoveryCouplingMode;
use crate::transport::webrtc::recovery_coordinator::RecoveryCouplingState;
use crate::transport::webrtc::startup_recovery::SessionPhase;
use crate::{XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig};

pub(crate) struct BweDecision {
    pub(crate) target_kbps: u32,
    pub(crate) reason: String,
}

pub(crate) struct TwccGccInput<'a> {
    pub(crate) observation: &'a XbxEngineVideoTwccObservation,
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
    let twcc_input = twcc_observation.map(|observation| TwccGccInput { observation });

    let (next_kbps, reason) = match config.bwe_mode.as_str() {
        "twcc-gcc" => resolve_twcc_gcc_target(
            config,
            current_kbps,
            actual_headroom_kbps,
            session_target_type,
            transport_path,
            session_phase,
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
                    session_target_type,
                    transport_path,
                    session_phase,
                    recovery_coupling,
                    Some(twcc),
                    hybrid_ramp_cooldown_ticks,
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
                            .min(actual_headroom_kbps.saturating_add(config.remb_ramp_up_step_kbps))
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
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
    session_phase: SessionPhase,
    recovery_coupling: Option<RecoveryCouplingState>,
    twcc_input: Option<&TwccGccInput<'_>>,
    ramp_cooldown_ticks: &mut u8,
) -> (u32, String) {
    let floor_kbps = config.remb_floor_kbps.max(1);
    let ceiling_kbps = config.remb_ceiling_kbps.max(floor_kbps);
    let profile = ScenarioPolicyResolver::resolve_transport_bwe_profile(
        config,
        session_target_type,
        transport_path,
        session_phase,
    );
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
    let stable_feedback = twcc.feedback_interval_ms.unwrap_or(0.0)
        <= profile.stable_feedback_interval_ms
        && twcc.observed_packet_count >= profile.stable_feedback_min_packets
        && twcc.covered_sequence_span >= twcc.observed_packet_count;
    let receive_bitrate_kbps = twcc
        .receive_bitrate_kbps
        .unwrap_or(actual_headroom_kbps as f64)
        .clamp(floor_kbps as f64, ceiling_kbps as f64) as u32;
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
        return (
            current_kbps
                .max(hold_floor_kbps)
                .clamp(preferred_gaming_floor_kbps, hold_ceiling_kbps),
            profile.reason(reason_suffix),
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

#[cfg(test)]
mod tests {
    use super::{
        classify_scenario_bitrate_band, resolve_transport_policy_profile_kind,
        resolve_twcc_gcc_target, RecoveryCouplingState, SessionPhase,
    };
    use crate::transport::webrtc::recovery_coordinator::RecoveryCouplingMode;
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
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Recovering,
            Some(coupling),
            Some(&super::TwccGccInput {
                observation: &observation,
            }),
            &mut cooldown,
        );
        assert_eq!(target, 23_000);
        assert_eq!(reason, "twcc-gcc-direct-recovery-coupled-reference-hold");
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
            Some(&XbxEngineTargetTypeDto::Cloud),
            Some("Direct (host->host)"),
            SessionPhase::Startup,
            Some(coupling),
            Some(&super::TwccGccInput {
                observation: &observation,
            }),
            &mut cooldown,
        );
        assert_eq!(target, 20_000);
        assert_eq!(reason, "twcc-gcc-cloud-recovery-coupled-wait-keyframe-hold");
    }
}
