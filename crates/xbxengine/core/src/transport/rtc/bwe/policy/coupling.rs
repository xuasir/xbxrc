use crate::transport::rtc::recovery::policy::TransportBweScenarioProfile;
use crate::transport::rtc::recovery::runtime_state::{RecoveryCouplingMode, RecoveryCouplingState};
use crate::{XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig};

pub(crate) struct CoupledHoldContext<'a> {
    pub(crate) config: &'a XbxEngineWebRtcRuntimeConfig,
    pub(crate) twcc: &'a XbxEngineVideoTwccObservation,
    pub(crate) profile: TransportBweScenarioProfile,
    pub(crate) coupling: RecoveryCouplingState,
    pub(crate) bounded_gaming_profile: bool,
    pub(crate) current_kbps: u32,
    pub(crate) actual_headroom_kbps: u32,
    pub(crate) desired_kbps: u32,
    pub(crate) raw_receive_headroom_kbps: u32,
    pub(crate) receive_headroom_kbps: u32,
    pub(crate) floor_kbps: u32,
    pub(crate) ceiling_kbps: u32,
    pub(crate) preferred_gaming_floor_kbps: u32,
    pub(crate) operating_ceiling_kbps: u32,
    pub(crate) effective_peak_ceiling_kbps: u32,
}

pub(crate) fn resolve_coupled_hold_target(
    context: CoupledHoldContext<'_>,
    ramp_cooldown_ticks: &mut u8,
) -> (u32, String) {
    let CoupledHoldContext {
        config,
        twcc,
        profile,
        coupling,
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
    } = context;

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
    if matches!(coupling.mode, RecoveryCouplingMode::StartupLowQuality)
        && matches!(
            profile.kind,
            crate::transport::rtc::recovery::policy::ScenarioPolicyProfileKind::CloudGaming
        )
        && (twcc.packet_loss_ratio >= profile.severe_loss_threshold
            || twcc.delivery_ratio <= profile.severe_delivery_threshold)
    {
        *ramp_cooldown_ticks = profile.congestion_cooldown_ticks.max(2);
        return (
            raw_receive_headroom_kbps.min(desired_kbps.max(1)),
            profile.reason("recovery-coupled-startup-backoff"),
        );
    }

    // reference chain 恢复遇到明显拥塞时，先退回保守 backoff。
    if matches!(
        coupling.mode,
        RecoveryCouplingMode::RecoveringReferenceChain
    ) && (twcc.packet_loss_ratio >= profile.congestion_loss_threshold
        || twcc.delivery_ratio <= profile.congestion_delivery_threshold)
    {
        *ramp_cooldown_ticks = profile.congestion_cooldown_ticks.max(1);
        let recovery_backoff_floor_kbps = profile.preferred_floor(floor_kbps);
        let backoff_kbps = ((current_kbps as f64) * (config.remb_ramp_down_factor as f64 / 1000.0))
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

    (
        current_kbps
            .max(hold_floor_kbps)
            .clamp(preferred_gaming_floor_kbps, hold_ceiling_kbps),
        profile.reason(reason_suffix),
    )
}
