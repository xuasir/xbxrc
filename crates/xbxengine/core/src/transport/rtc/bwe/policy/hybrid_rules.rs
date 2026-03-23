use crate::transport::rtc::bwe::policy::twcc_rules::{is_high_rtt, is_severe_rtt};
use crate::transport::rtc::recovery::policy::TransportBweScenarioProfile;
use crate::XbxEngineWebRtcRuntimeConfig;

pub(crate) struct HybridRuleContext<'a> {
    pub(crate) config: &'a XbxEngineWebRtcRuntimeConfig,
    pub(crate) transport_profile: TransportBweScenarioProfile,
    pub(crate) current_kbps: u32,
    pub(crate) observed_kbps: u32,
    pub(crate) actual_kbps: f64,
    pub(crate) actual_headroom_kbps: u32,
    pub(crate) loss_ratio: f64,
    pub(crate) rtt_ms: Option<f64>,
    pub(crate) observed_remb_present: bool,
    pub(crate) floor_kbps: u32,
    pub(crate) ceiling_kbps: u32,
}

pub(crate) fn resolve_hybrid_target(
    context: HybridRuleContext<'_>,
    ramp_cooldown_ticks: &mut u8,
) -> (u32, String) {
    if is_severe_rtt(context.transport_profile, context.rtt_ms) {
        *ramp_cooldown_ticks = context.transport_profile.severe_cooldown_ticks;
        return (
            ((context.current_kbps as f64) * (context.config.remb_ramp_down_factor as f64 / 1000.0))
                .round()
                .max(context.floor_kbps as f64)
                .min(context.ceiling_kbps as f64) as u32,
            "hybrid-severe-rtt-backoff".to_string(),
        );
    }

    if is_high_rtt(context.transport_profile, context.rtt_ms) {
        *ramp_cooldown_ticks = context.transport_profile.congestion_cooldown_ticks.max(1);
        return (
            context
                .current_kbps
                .clamp(context.floor_kbps, context.ceiling_kbps),
            "hybrid-high-rtt-hold".to_string(),
        );
    }

    let severe_loss = context.loss_ratio >= 0.08;
    let sustained_loss = context.loss_ratio >= 0.01;
    let mild_loss = context.loss_ratio >= 0.005;
    let bitrate_overrun = context.actual_kbps > (context.current_kbps as f64 * 1.1);

    if severe_loss || bitrate_overrun {
        *ramp_cooldown_ticks = 12;
        return (
            ((context.current_kbps as f64) * (context.config.remb_ramp_down_factor as f64 / 1000.0))
                .round()
                .max(context.floor_kbps as f64) as u32,
            if severe_loss {
                "hybrid-severe-loss-backoff".to_string()
            } else {
                "hybrid-bitrate-overrun-backoff".to_string()
            },
        );
    }

    if sustained_loss {
        *ramp_cooldown_ticks = 10;
        return (
            context
                .current_kbps
                .min(context.actual_headroom_kbps)
                .max(context.floor_kbps),
            "hybrid-sustained-loss-cap".to_string(),
        );
    }

    if mild_loss {
        *ramp_cooldown_ticks = 6;
        return (
            context
                .current_kbps
                .min(
                    context
                        .actual_headroom_kbps
                        .saturating_add(context.config.remb_ramp_up_step_kbps),
                )
                .max(context.floor_kbps),
            "hybrid-mild-loss-hold".to_string(),
        );
    }

    if *ramp_cooldown_ticks > 0 {
        *ramp_cooldown_ticks = ramp_cooldown_ticks.saturating_sub(1);
        return (context.current_kbps, "hybrid-ramp-cooldown".to_string());
    }

    let desired_kbps = context.observed_kbps.min(context.ceiling_kbps);
    (
        context
            .current_kbps
            .saturating_add(context.config.remb_ramp_up_step_kbps)
            .min(desired_kbps)
            .max(context.floor_kbps),
        if context.observed_remb_present {
            "hybrid-ramp-up-observed".to_string()
        } else {
            "hybrid-ramp-up-ceiling".to_string()
        },
    )
}
