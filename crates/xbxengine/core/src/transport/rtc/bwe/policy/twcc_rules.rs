use crate::transport::rtc::recovery::policy::{
    ScenarioPolicyProfileKind, TransportBweScenarioProfile,
};
use crate::{XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig};

#[derive(Clone, Copy)]
pub(crate) struct TwccRuleContext<'a> {
    pub(crate) config: &'a XbxEngineWebRtcRuntimeConfig,
    pub(crate) twcc: &'a XbxEngineVideoTwccObservation,
    pub(crate) profile: TransportBweScenarioProfile,
    pub(crate) bounded_gaming_profile: bool,
    pub(crate) current_kbps: u32,
    pub(crate) actual_headroom_kbps: u32,
    pub(crate) desired_kbps: u32,
    pub(crate) receive_headroom_kbps: u32,
    pub(crate) floor_kbps: u32,
    pub(crate) ceiling_kbps: u32,
    pub(crate) preferred_gaming_floor_kbps: u32,
    pub(crate) effective_peak_ceiling_kbps: u32,
    pub(crate) peak_enter_kbps: u32,
    pub(crate) ramp_up_step_kbps: u32,
    pub(crate) fast_ramp_up_step_kbps: u32,
}

pub(crate) fn resolve_rtt_rule(
    context: TwccRuleContext<'_>,
    rtt_ms: Option<f64>,
    ramp_cooldown_ticks: &mut u8,
) -> Option<(u32, String)> {
    if is_severe_rtt(context.profile, rtt_ms) {
        *ramp_cooldown_ticks = context.profile.severe_cooldown_ticks;
        let backoff_kbps = ((context.current_kbps as f64)
            * (context.config.remb_ramp_down_factor as f64 / 1000.0))
            .round()
            .max(context.floor_kbps as f64) as u32;
        return Some((
            if context.bounded_gaming_profile {
                backoff_kbps
                    .min(
                        context
                            .desired_kbps
                            .max(context.preferred_gaming_floor_kbps),
                    )
                    .max(context.preferred_gaming_floor_kbps)
            } else {
                backoff_kbps.min(context.receive_headroom_kbps.max(context.floor_kbps))
            },
            context.profile.reason("severe-rtt-backoff"),
        ));
    }

    if is_high_rtt(context.profile, rtt_ms) {
        *ramp_cooldown_ticks = context.profile.congestion_cooldown_ticks.max(1);
        return Some((
            if context.bounded_gaming_profile {
                context
                    .current_kbps
                    .min(
                        context
                            .effective_peak_ceiling_kbps
                            .min(context.ceiling_kbps),
                    )
                    .max(context.preferred_gaming_floor_kbps)
            } else {
                context
                    .current_kbps
                    .min(context.receive_headroom_kbps.max(context.floor_kbps))
            },
            context.profile.reason("high-rtt-hold"),
        ));
    }

    None
}

pub(crate) fn resolve_loss_rule(
    context: TwccRuleContext<'_>,
    ramp_cooldown_ticks: &mut u8,
) -> Option<(u32, String)> {
    if context.twcc.packet_loss_ratio >= context.profile.severe_loss_threshold
        || context.twcc.delivery_ratio <= context.profile.severe_delivery_threshold
    {
        if should_apply_cloud_optimistic_loss_cap(context) {
            *ramp_cooldown_ticks = context.profile.congestion_cooldown_ticks.max(1);
            let cap_kbps = context
                .desired_kbps
                .min(
                    context
                        .effective_peak_ceiling_kbps
                        .min(context.ceiling_kbps),
                )
                .max(context.preferred_gaming_floor_kbps);
            return Some((
                context.current_kbps.min(cap_kbps),
                context.profile.reason("severe-optimistic-cap"),
            ));
        }
        *ramp_cooldown_ticks = context.profile.severe_cooldown_ticks;
        let backoff_kbps = ((context.current_kbps as f64)
            * (context.config.remb_ramp_down_factor as f64 / 1000.0))
            .round()
            .max(context.floor_kbps as f64) as u32;
        return Some((
            if context.bounded_gaming_profile {
                backoff_kbps
                    .min(
                        context
                            .desired_kbps
                            .max(context.preferred_gaming_floor_kbps),
                    )
                    .max(context.preferred_gaming_floor_kbps)
            } else {
                backoff_kbps.min(context.receive_headroom_kbps.max(context.floor_kbps))
            },
            context.profile.reason("severe-backoff"),
        ));
    }

    if context.twcc.packet_loss_ratio >= context.profile.congestion_loss_threshold
        || context.twcc.delivery_ratio <= context.profile.congestion_delivery_threshold
    {
        *ramp_cooldown_ticks = context.profile.congestion_cooldown_ticks;
        let cap_kbps = context
            .desired_kbps
            .min(
                context
                    .effective_peak_ceiling_kbps
                    .min(context.ceiling_kbps),
            )
            .max(context.preferred_gaming_floor_kbps);
        return Some((
            if context.bounded_gaming_profile {
                context.current_kbps.min(cap_kbps)
            } else {
                context
                    .current_kbps
                    .min(context.receive_headroom_kbps.max(context.floor_kbps))
            },
            context.profile.reason("congestion-cap"),
        ));
    }

    if context.twcc.packet_loss_ratio >= context.profile.mild_loss_threshold
        || context.twcc.delivery_ratio <= context.profile.mild_delivery_threshold
    {
        *ramp_cooldown_ticks = context.profile.mild_cooldown_ticks;
        return Some((
            if context.bounded_gaming_profile {
                context
                    .current_kbps
                    .min(
                        context
                            .effective_peak_ceiling_kbps
                            .min(context.ceiling_kbps),
                    )
                    .max(context.preferred_gaming_floor_kbps)
            } else {
                context
                    .current_kbps
                    .min(
                        context
                            .receive_headroom_kbps
                            .saturating_add(context.ramp_up_step_kbps),
                    )
                    .max(context.floor_kbps)
            },
            context.profile.reason("mild-hold"),
        ));
    }

    None
}

fn should_apply_cloud_optimistic_loss_cap(context: TwccRuleContext<'_>) -> bool {
    if !context.bounded_gaming_profile || context.profile.kind != ScenarioPolicyProfileKind::CloudGaming
    {
        return false;
    }
    let expected_interval_ms = context.config.video_pipeline.feedback_interval_ms.max(1) as f64;
    let Some(feedback_interval_ms) = context.twcc.feedback_interval_ms else {
        return false;
    };
    feedback_interval_ms >= expected_interval_ms * 1.6
}

pub(crate) fn resolve_cooldown_or_ramp(
    context: TwccRuleContext<'_>,
    ramp_cooldown_ticks: &mut u8,
) -> (u32, String) {
    if *ramp_cooldown_ticks > 0 {
        *ramp_cooldown_ticks = ramp_cooldown_ticks.saturating_sub(1);
        return (
            if context.bounded_gaming_profile {
                context
                    .current_kbps
                    .saturating_add(context.profile.cooldown_recovery_step_kbps.unwrap_or(0))
                    .min(context.desired_kbps)
                    .min(
                        context
                            .effective_peak_ceiling_kbps
                            .min(context.ceiling_kbps),
                    )
                    .max(context.preferred_gaming_floor_kbps)
            } else {
                context
                    .current_kbps
                    .max(context.preferred_gaming_floor_kbps)
            },
            context.profile.reason("ramp-cooldown"),
        );
    }

    let desired_kbps = if context.bounded_gaming_profile {
        context
            .desired_kbps
            .max(
                context.actual_headroom_kbps.min(
                    context
                        .effective_peak_ceiling_kbps
                        .min(context.ceiling_kbps),
                ),
            )
            .clamp(
                context.preferred_gaming_floor_kbps,
                context
                    .effective_peak_ceiling_kbps
                    .min(context.ceiling_kbps),
            )
    } else {
        context
            .receive_headroom_kbps
            .max(context.actual_headroom_kbps)
            .max(context.preferred_gaming_floor_kbps)
            .clamp(context.floor_kbps, context.ceiling_kbps)
    };

    (
        context
            .current_kbps
            .saturating_add(
                if context.bounded_gaming_profile && desired_kbps >= context.peak_enter_kbps {
                    context.fast_ramp_up_step_kbps
                } else {
                    context.ramp_up_step_kbps
                },
            )
            .min(desired_kbps)
            .max(context.floor_kbps),
        context.profile.reason("ramp-up"),
    )
}

pub(crate) fn is_high_rtt(profile: TransportBweScenarioProfile, rtt_ms: Option<f64>) -> bool {
    match (profile.high_rtt_ms_threshold, rtt_ms) {
        (Some(threshold_ms), Some(value_ms)) => value_ms >= threshold_ms,
        _ => false,
    }
}

pub(crate) fn is_severe_rtt(profile: TransportBweScenarioProfile, rtt_ms: Option<f64>) -> bool {
    match (profile.severe_rtt_ms_threshold, rtt_ms) {
        (Some(threshold_ms), Some(value_ms)) => value_ms >= threshold_ms,
        _ => false,
    }
}
