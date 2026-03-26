#[path = "policy/coupling.rs"]
mod coupling;
#[path = "policy/hybrid_rules.rs"]
mod hybrid_rules;
#[path = "policy/twcc_rules.rs"]
mod twcc_rules;

use xbxengine_protocol::XbxEngineTargetTypeDto;

use self::coupling::{resolve_coupled_hold_target, CoupledHoldContext};
use self::hybrid_rules::{resolve_hybrid_target, HybridRuleContext};
use self::twcc_rules::{
    resolve_cooldown_or_ramp, resolve_loss_rule, resolve_rtt_rule, TwccRuleContext,
};
use crate::transport::rtc::recovery::policy::{ScenarioPolicyProfileKind, ScenarioPolicyResolver};
use crate::transport::rtc::recovery::runtime_state::RecoveryCouplingState;
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
    let effective_stable_feedback_interval_ms = profile
        .stable_feedback_interval_ms
        .max(config.video_pipeline.feedback_interval_ms as f64 * 1.25);
    let stable_feedback = twcc.feedback_interval_ms.unwrap_or(0.0)
        <= effective_stable_feedback_interval_ms
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

    if bounded_gaming_profile
        && recovery_coupling
            .as_ref()
            .is_some_and(|coupling| coupling.suppress_ramp_up || coupling.prefer_hold)
    {
        return resolve_coupled_hold_target(
            CoupledHoldContext {
                config,
                twcc,
                profile,
                coupling: recovery_coupling.expect("checked above"),
                bounded_gaming_profile,
                current_kbps,
                actual_headroom_kbps,
                desired_kbps,
                raw_receive_headroom_kbps,
                receive_headroom_kbps,
                floor_kbps,
                ceiling_kbps,
                preferred_gaming_floor_kbps,
                operating_ceiling_kbps,
                effective_peak_ceiling_kbps,
            },
            ramp_cooldown_ticks,
        );
    }

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
mod tests {
    use super::{
        classify_scenario_bitrate_band, resolve_target_remb_kbps,
        resolve_transport_policy_profile_kind, resolve_twcc_gcc_target, RecoveryCouplingState,
        SessionPhase,
    };
    use crate::transport::rtc::recovery::policy::ScenarioPolicyResolver;
    use crate::transport::rtc::recovery::runtime_state::RecoveryCouplingMode;
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
            source: "local-feedback".to_string(),
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
            source: "local-feedback".to_string(),
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
            source: "local-feedback".to_string(),
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
            source: "local-feedback".to_string(),
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
            source: "local-feedback".to_string(),
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
    fn cloud_normal_rtt_does_not_hold_clean_twcc_feedback() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 3,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 160,
            covered_sequence_span: 160,
            observed_packet_count: 160,
            observed_byte_count: 220_000,
            feedback_interval_ms: Some(1_000.0),
            arrival_span_ms: Some(1_000.0),
            receive_bitrate_kbps: Some(24_000.0),
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            25_000,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct"),
                SessionPhase::Steady,
            ),
            None,
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(200.0),
            }),
            &mut cooldown,
        );

        assert_eq!(target, 27_360);
        assert_eq!(reason, "twcc-gcc-cloud-ramp-up");
        assert_eq!(cooldown, 0);
    }

    #[test]
    fn cloud_very_high_rtt_still_holds_clean_twcc_feedback() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 4,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 160,
            covered_sequence_span: 160,
            observed_packet_count: 160,
            observed_byte_count: 220_000,
            feedback_interval_ms: Some(1_000.0),
            arrival_span_ms: Some(1_000.0),
            receive_bitrate_kbps: Some(24_000.0),
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            25_000,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct"),
                SessionPhase::Steady,
            ),
            None,
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(330.0),
            }),
            &mut cooldown,
        );

        assert_eq!(target, 25_000);
        assert_eq!(reason, "twcc-gcc-cloud-high-rtt-hold");
        assert_eq!(cooldown, 2);
    }

    #[test]
    fn cloud_long_feedback_interval_can_still_be_treated_as_stable() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 5,
            source: "local-feedback".to_string(),
            feedback_packet_count: 40,
            covered_sequence_start: 1,
            covered_sequence_end: 826,
            covered_sequence_span: 826,
            observed_packet_count: 798,
            observed_byte_count: 957_600,
            feedback_interval_ms: Some(4_042.0),
            arrival_span_ms: Some(999.0),
            receive_bitrate_kbps: Some(1_895.299356754082),
            delivery_ratio: 0.9661016949152542,
            packet_loss_ratio: 0.03389830508474578,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            28_500,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct"),
                SessionPhase::Steady,
            ),
            None,
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(194.0),
            }),
            &mut cooldown,
        );

        assert_ne!(reason, "twcc-gcc-cloud-unstable-hold");
        assert_eq!(target, 28_500);
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

    #[test]
    fn hybrid_without_twcc_high_rtt_holds_current_target() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "hybrid".to_string();
        let mut last_sent = 18_000;
        let mut cooldown = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            16_000.0,
            0.0,
            Some(90.0),
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            None,
            &mut last_sent,
            &mut cooldown,
        );

        assert_eq!(decision.target_kbps, 18_000);
        assert_eq!(decision.reason, "hybrid-high-rtt-hold");
        assert_eq!(last_sent, 18_000);
        assert_eq!(cooldown, 1);
    }

    #[test]
    fn hybrid_without_twcc_sustained_loss_caps_to_actual_headroom() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "hybrid".to_string();
        let mut last_sent = 20_000;
        let mut cooldown = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            Some(24_000),
            12_000.0,
            0.02,
            None,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            None,
            &mut last_sent,
            &mut cooldown,
        );

        assert_eq!(decision.target_kbps, 15_000);
        assert_eq!(decision.reason, "hybrid-sustained-loss-cap");
        assert_eq!(last_sent, 15_000);
        assert_eq!(cooldown, 10);
    }

    #[test]
    fn hybrid_without_twcc_cooldown_then_ramps_up_to_observed_remb() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "hybrid".to_string();
        let mut last_sent = 14_000;
        let mut cooldown = 1;

        let first = resolve_target_remb_kbps(
            &config,
            Some(24_000),
            14_000.0,
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
        assert_eq!(first.target_kbps, 14_000);
        assert_eq!(first.reason, "hybrid-ramp-cooldown");
        assert_eq!(cooldown, 0);

        let second = resolve_target_remb_kbps(
            &config,
            Some(24_000),
            14_000.0,
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
        assert_eq!(second.target_kbps, 14_000 + config.remb_ramp_up_step_kbps);
        assert_eq!(second.reason, "hybrid-ramp-up-observed");
        assert_eq!(last_sent, second.target_kbps);
    }
}
