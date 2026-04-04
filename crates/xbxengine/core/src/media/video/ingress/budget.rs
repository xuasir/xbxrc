use std::time::Duration;

use crate::media::video::types::{AssembledVideoFrame, EncodedFrame, FrameValue};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FrameBudgetRecoveryPhase {
    #[default]
    Steady,
    Repairing,
    AwaitingKeyframe,
    Reconfiguring,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FrameBudgetLinkValue {
    #[default]
    Disposable,
    Supply,
    Anchor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FrameBudgetRttSlack {
    #[default]
    Unknown,
    Ample,
    Tight,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FrameBudgetFailureCost {
    #[default]
    LocalDrop,
    WaitKeyframe,
    Reconfigure,
    ChainBroken,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FrameBudgetWindowSource {
    #[default]
    Playout,
    Transport,
    Recovery,
    Reconfigure,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameBudgetContext {
    pub(crate) recovery_phase: FrameBudgetRecoveryPhase,
    pub(crate) link_value: FrameBudgetLinkValue,
    pub(crate) rtt_slack: FrameBudgetRttSlack,
    pub(crate) failure_cost: FrameBudgetFailureCost,
    pub(crate) window_source: FrameBudgetWindowSource,
}

impl FrameBudgetContext {
    pub(crate) fn steady_for_value(value: FrameValue) -> Self {
        Self {
            link_value: resolve_link_value(
                value,
                FrameBudgetRecoveryPhase::Steady,
                FrameBudgetFailureCost::LocalDrop,
            ),
            ..Self::default()
        }
    }

    pub(crate) fn for_ingress_materialization_parts(
        value: FrameValue,
        frame_playout_deadline_at_ms: Option<f64>,
        frame_unrecoverable_reason: Option<&str>,
    ) -> Self {
        let failure_cost = match frame_unrecoverable_reason {
            Some("referenceChainUnrecoverable" | "awaitingRecoveryKeyframe") => {
                FrameBudgetFailureCost::ChainBroken
            }
            Some(
                "parameterSetsChanged" | "dimensionsChanged" | "codecChanged" | "configChanged",
            ) => FrameBudgetFailureCost::Reconfigure,
            _ => FrameBudgetFailureCost::LocalDrop,
        };
        let recovery_phase = match failure_cost {
            FrameBudgetFailureCost::ChainBroken => FrameBudgetRecoveryPhase::AwaitingKeyframe,
            FrameBudgetFailureCost::Reconfigure => FrameBudgetRecoveryPhase::Reconfiguring,
            FrameBudgetFailureCost::LocalDrop if frame_playout_deadline_at_ms.is_some() => {
                FrameBudgetRecoveryPhase::Repairing
            }
            _ => FrameBudgetRecoveryPhase::Steady,
        };
        let window_source = match failure_cost {
            FrameBudgetFailureCost::Reconfigure => FrameBudgetWindowSource::Reconfigure,
            _ if frame_playout_deadline_at_ms.is_some() => FrameBudgetWindowSource::Recovery,
            _ => FrameBudgetWindowSource::Playout,
        };
        Self {
            recovery_phase,
            link_value: resolve_link_value(value, recovery_phase, failure_cost),
            rtt_slack: FrameBudgetRttSlack::Unknown,
            failure_cost,
            window_source,
        }
    }

    pub(crate) fn for_ingress_admission(
        frame: &EncodedFrame,
        waiting_keyframe: bool,
        config_mismatch: bool,
    ) -> Self {
        let failure_cost = if matches!(
            frame.frame_unrecoverable_reason.as_deref(),
            Some("referenceChainUnrecoverable" | "awaitingRecoveryKeyframe")
        ) {
            FrameBudgetFailureCost::ChainBroken
        } else if waiting_keyframe {
            FrameBudgetFailureCost::WaitKeyframe
        } else if config_mismatch || frame.config_changed || frame.h264.parameter_sets_changed {
            FrameBudgetFailureCost::Reconfigure
        } else {
            FrameBudgetFailureCost::LocalDrop
        };
        let recovery_phase = match failure_cost {
            FrameBudgetFailureCost::ChainBroken | FrameBudgetFailureCost::WaitKeyframe => {
                FrameBudgetRecoveryPhase::AwaitingKeyframe
            }
            FrameBudgetFailureCost::Reconfigure => FrameBudgetRecoveryPhase::Reconfiguring,
            FrameBudgetFailureCost::LocalDrop if frame.frame_playout_deadline_at_ms.is_some() => {
                FrameBudgetRecoveryPhase::Repairing
            }
            _ => FrameBudgetRecoveryPhase::Steady,
        };
        let window_source = match failure_cost {
            FrameBudgetFailureCost::Reconfigure => FrameBudgetWindowSource::Reconfigure,
            _ if frame.frame_playout_deadline_at_ms.is_some() => FrameBudgetWindowSource::Recovery,
            _ => FrameBudgetWindowSource::Playout,
        };
        Self {
            recovery_phase,
            link_value: resolve_link_value(frame.value, recovery_phase, failure_cost),
            rtt_slack: FrameBudgetRttSlack::Unknown,
            failure_cost,
            window_source,
        }
    }

    pub(crate) fn for_transport(
        value: FrameValue,
        waiting_keyframe: bool,
        cloud_rtt_ms: Option<f64>,
        estimated_recovery_arrival_ms: Option<f64>,
        deadline_at_ms: Option<f64>,
        startup_mode: bool,
        window_source: FrameBudgetWindowSource,
    ) -> Self {
        let rtt_slack =
            resolve_rtt_slack(cloud_rtt_ms, estimated_recovery_arrival_ms, deadline_at_ms);
        let failure_cost = if waiting_keyframe && !value.is_sync_point() {
            FrameBudgetFailureCost::ChainBroken
        } else if waiting_keyframe
            || (startup_mode
                && matches!(
                    rtt_slack,
                    FrameBudgetRttSlack::Tight | FrameBudgetRttSlack::Exhausted
                )
                && !value.is_sync_point())
        {
            FrameBudgetFailureCost::WaitKeyframe
        } else {
            FrameBudgetFailureCost::LocalDrop
        };
        let recovery_phase = if waiting_keyframe {
            FrameBudgetRecoveryPhase::AwaitingKeyframe
        } else if matches!(window_source, FrameBudgetWindowSource::Recovery) {
            FrameBudgetRecoveryPhase::Repairing
        } else {
            FrameBudgetRecoveryPhase::Steady
        };
        Self {
            recovery_phase,
            link_value: resolve_link_value(value, recovery_phase, failure_cost),
            rtt_slack,
            failure_cost,
            window_source,
        }
    }

    pub(crate) fn late_budget_ratio_per_mille(&self, value: FrameValue) -> u16 {
        adjust_budget_ratio(value.late_budget_ratio_per_mille(), *self, value, false)
    }

    pub(crate) fn deadline_budget_ratio_per_mille(&self, value: FrameValue) -> u16 {
        adjust_budget_ratio(value.deadline_budget_ratio_per_mille(), *self, value, true)
    }

    pub(crate) fn backlog_priority_score(&self, value: FrameValue) -> u32 {
        let mut score = value.backlog_priority_score() as i32;
        score += match self.link_value {
            FrameBudgetLinkValue::Anchor => 700,
            FrameBudgetLinkValue::Supply => 260,
            FrameBudgetLinkValue::Disposable => 0,
        };
        score += match self.failure_cost {
            FrameBudgetFailureCost::ChainBroken => 600,
            FrameBudgetFailureCost::Reconfigure => 320,
            FrameBudgetFailureCost::WaitKeyframe => 220,
            FrameBudgetFailureCost::LocalDrop => 0,
        };
        score += match self.recovery_phase {
            FrameBudgetRecoveryPhase::AwaitingKeyframe => 260,
            FrameBudgetRecoveryPhase::Repairing => 140,
            FrameBudgetRecoveryPhase::Reconfiguring => 180,
            FrameBudgetRecoveryPhase::Steady => 0,
        };
        score -= match self.rtt_slack {
            FrameBudgetRttSlack::Exhausted => 220,
            FrameBudgetRttSlack::Tight => 100,
            FrameBudgetRttSlack::Ample | FrameBudgetRttSlack::Unknown => 0,
        };
        score.max(0) as u32
    }

    pub(crate) fn frame_importance(&self) -> &'static str {
        match self.link_value {
            FrameBudgetLinkValue::Anchor => "keyframe",
            FrameBudgetLinkValue::Supply => "reference",
            FrameBudgetLinkValue::Disposable => "delta",
        }
    }

    pub(crate) fn repair_priority(&self, value: FrameValue) -> u8 {
        let base: u8 = match self.link_value {
            FrameBudgetLinkValue::Anchor => 3,
            FrameBudgetLinkValue::Supply => 2,
            FrameBudgetLinkValue::Disposable => 1,
        };
        let failure_bonus: u8 = match self.failure_cost {
            FrameBudgetFailureCost::ChainBroken => 1,
            FrameBudgetFailureCost::Reconfigure
            | FrameBudgetFailureCost::WaitKeyframe
            | FrameBudgetFailureCost::LocalDrop => 0,
        };
        let sync_bonus = u8::from(value.is_sync_point());
        base.saturating_add(failure_bonus)
            .saturating_add(sync_bonus)
            .min(4)
    }

    pub(crate) fn retry_budget(&self, value: FrameValue, default_max_retry_count: u8) -> u8 {
        match (self.link_value, self.failure_cost, self.rtt_slack) {
            (_, FrameBudgetFailureCost::ChainBroken, FrameBudgetRttSlack::Exhausted) => 0,
            (FrameBudgetLinkValue::Anchor, _, _) => default_max_retry_count.min(1),
            (FrameBudgetLinkValue::Supply, FrameBudgetFailureCost::ChainBroken, _) => {
                default_max_retry_count.min(1)
            }
            (FrameBudgetLinkValue::Supply, _, FrameBudgetRttSlack::Exhausted) => 0,
            (FrameBudgetLinkValue::Supply, _, _) if value.refresh_boost => {
                default_max_retry_count.min(1)
            }
            (FrameBudgetLinkValue::Supply, _, _) => 0,
            (FrameBudgetLinkValue::Disposable, _, _) => 0,
        }
    }

    pub(crate) fn prefers_low_value_skip(&self) -> bool {
        matches!(self.link_value, FrameBudgetLinkValue::Disposable)
            && matches!(
                self.rtt_slack,
                FrameBudgetRttSlack::Tight | FrameBudgetRttSlack::Exhausted
            )
            && matches!(self.failure_cost, FrameBudgetFailureCost::LocalDrop)
    }

    pub(crate) fn prefers_chain_broken(&self) -> bool {
        matches!(self.failure_cost, FrameBudgetFailureCost::ChainBroken)
    }

    pub(crate) fn prefers_wait_keyframe(&self) -> bool {
        matches!(
            self.failure_cost,
            FrameBudgetFailureCost::WaitKeyframe | FrameBudgetFailureCost::ChainBroken
        )
    }

    pub(crate) fn prefers_reconfigure(&self) -> bool {
        matches!(self.failure_cost, FrameBudgetFailureCost::Reconfigure)
    }
}

// playout budget 属于 decode 前的准入语义，应放在 ingress 侧统一定义。
pub fn materialize_ingress_frame(
    frame: AssembledVideoFrame,
    min_delay: Duration,
    max_delay: Duration,
) -> EncodedFrame {
    let context = frame.budget;
    materialize_ingress_frame_with_context(frame, min_delay, max_delay, context)
}

pub(crate) fn materialize_ingress_frame_with_context(
    frame: AssembledVideoFrame,
    min_delay: Duration,
    max_delay: Duration,
    context: FrameBudgetContext,
) -> EncodedFrame {
    let playout_delay = resolve_playout_delay(frame.value, min_delay, max_delay, context);
    let target_playout_time = frame.assembled_at + playout_delay;
    frame.into_encoded_frame(target_playout_time)
}

fn resolve_playout_delay(
    value: FrameValue,
    min_delay: Duration,
    max_delay: Duration,
    context: FrameBudgetContext,
) -> Duration {
    if matches!(context.link_value, FrameBudgetLinkValue::Anchor)
        || matches!(context.failure_cost, FrameBudgetFailureCost::ChainBroken)
    {
        return max_delay.max(min_delay);
    }

    let ratio = context.deadline_budget_ratio_per_mille(value) as u128;
    let min_ms = min_delay.as_millis();
    let max_ms = max_delay.as_millis().max(min_ms);
    let spread_ms = max_ms.saturating_sub(min_ms);
    let scaled_ms = min_ms + (spread_ms * ratio / 1_000);
    Duration::from_millis(scaled_ms as u64).max(min_delay)
}

fn resolve_link_value(
    value: FrameValue,
    recovery_phase: FrameBudgetRecoveryPhase,
    failure_cost: FrameBudgetFailureCost,
) -> FrameBudgetLinkValue {
    let base = if value.is_sync_point() {
        FrameBudgetLinkValue::Anchor
    } else if value.refresh_boost {
        FrameBudgetLinkValue::Supply
    } else {
        FrameBudgetLinkValue::Disposable
    };

    match (base, recovery_phase, failure_cost) {
        (FrameBudgetLinkValue::Anchor, _, _) => FrameBudgetLinkValue::Anchor,
        (FrameBudgetLinkValue::Supply, FrameBudgetRecoveryPhase::AwaitingKeyframe, _) => {
            FrameBudgetLinkValue::Anchor
        }
        (
            FrameBudgetLinkValue::Disposable,
            FrameBudgetRecoveryPhase::AwaitingKeyframe | FrameBudgetRecoveryPhase::Repairing,
            FrameBudgetFailureCost::ChainBroken,
        ) => FrameBudgetLinkValue::Supply,
        (
            FrameBudgetLinkValue::Disposable,
            FrameBudgetRecoveryPhase::Reconfiguring,
            FrameBudgetFailureCost::Reconfigure,
        ) => FrameBudgetLinkValue::Supply,
        _ => base,
    }
}

fn resolve_rtt_slack(
    cloud_rtt_ms: Option<f64>,
    estimated_recovery_arrival_ms: Option<f64>,
    deadline_at_ms: Option<f64>,
) -> FrameBudgetRttSlack {
    if let Some(slack_ms) = deadline_at_ms
        .zip(estimated_recovery_arrival_ms)
        .map(|(deadline, arrival)| deadline - arrival)
    {
        if slack_ms <= 0.0 {
            return FrameBudgetRttSlack::Exhausted;
        }
        if slack_ms <= 12.0 {
            return FrameBudgetRttSlack::Tight;
        }
        return FrameBudgetRttSlack::Ample;
    }
    match cloud_rtt_ms.unwrap_or_default() {
        rtt if rtt >= 180.0 => FrameBudgetRttSlack::Exhausted,
        rtt if rtt >= 120.0 => FrameBudgetRttSlack::Tight,
        rtt if rtt > 0.0 => FrameBudgetRttSlack::Ample,
        _ => FrameBudgetRttSlack::Unknown,
    }
}

fn adjust_budget_ratio(
    base_ratio_per_mille: u16,
    context: FrameBudgetContext,
    value: FrameValue,
    deadline_budget: bool,
) -> u16 {
    let mut ratio = i32::from(base_ratio_per_mille);
    ratio += match context.link_value {
        FrameBudgetLinkValue::Anchor => 280,
        FrameBudgetLinkValue::Supply => 120,
        FrameBudgetLinkValue::Disposable => 0,
    };
    ratio += match context.failure_cost {
        FrameBudgetFailureCost::ChainBroken => 260,
        FrameBudgetFailureCost::Reconfigure => 150,
        FrameBudgetFailureCost::WaitKeyframe => 120,
        FrameBudgetFailureCost::LocalDrop => 0,
    };
    ratio += match context.recovery_phase {
        FrameBudgetRecoveryPhase::AwaitingKeyframe => {
            if matches!(context.link_value, FrameBudgetLinkValue::Disposable) {
                -120
            } else {
                180
            }
        }
        FrameBudgetRecoveryPhase::Repairing => 80,
        FrameBudgetRecoveryPhase::Reconfiguring => 120,
        FrameBudgetRecoveryPhase::Steady => 0,
    };
    ratio += match context.rtt_slack {
        FrameBudgetRttSlack::Exhausted => {
            if deadline_budget && !value.is_sync_point() {
                -240
            } else {
                -120
            }
        }
        FrameBudgetRttSlack::Tight => {
            if deadline_budget && !value.is_sync_point() {
                -120
            } else {
                -60
            }
        }
        FrameBudgetRttSlack::Ample | FrameBudgetRttSlack::Unknown => 0,
    };
    ratio += match context.window_source {
        FrameBudgetWindowSource::Recovery => 100,
        FrameBudgetWindowSource::Reconfigure => 140,
        FrameBudgetWindowSource::Transport | FrameBudgetWindowSource::Playout => 0,
    };
    ratio.clamp(250, 1_600) as u16
}

#[cfg(test)]
mod tests {
    use super::{
        materialize_ingress_frame, FrameBudgetContext, FrameBudgetFailureCost,
        FrameBudgetLinkValue, FrameBudgetRecoveryPhase, FrameBudgetWindowSource,
    };
    use crate::media::video::h264::inspection::{
        H264AccessUnitInspection, H264BootstrapRejectReason,
    };
    use crate::media::video::types::{
        AssembledVideoFrame, FrameRecoveryDisposition, FrameValue, VideoCodec,
    };
    use bytes::Bytes;
    use std::time::{Duration, Instant};

    fn make_h264_inspection(bootstrap_ready: bool) -> H264AccessUnitInspection {
        H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: Some(1920),
            height: Some(1080),
            is_idr: bootstrap_ready,
            has_inband_sps: bootstrap_ready,
            has_inband_pps: bootstrap_ready,
            slice_headers_valid: bootstrap_ready,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready,
            bootstrap_reject_reason: if bootstrap_ready {
                None
            } else {
                Some(H264BootstrapRejectReason::MissingSps)
            },
            commit_state:
                crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
        }
    }

    #[test]
    fn delta_frame_gets_tighter_playout_budget_than_keyframe() {
        let assembled_at = Instant::now();
        let keyframe = materialize_ingress_frame(
            AssembledVideoFrame {
                codec: VideoCodec::H264,
                is_keyframe: true,
                config_changed: true,
                value: FrameValue::new(true, true, 64 * 1024),
                budget: FrameBudgetContext::for_ingress_materialization_parts(
                    FrameValue::new(true, true, 64 * 1024),
                    None,
                    None,
                ),
                width: 1920,
                height: 1080,
                rtp_timestamp: 1,
                frame_playout_deadline_at_ms: None,
                frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
                frame_unrecoverable_reason: None,
                assembled_at,
                h264: make_h264_inspection(true),
                payload: Bytes::from_static(b"k"),
            },
            Duration::from_millis(8),
            Duration::from_millis(30),
        );
        let delta = materialize_ingress_frame(
            AssembledVideoFrame {
                codec: VideoCodec::H264,
                is_keyframe: false,
                config_changed: false,
                value: FrameValue::new(false, false, 8 * 1024),
                budget: FrameBudgetContext::for_ingress_materialization_parts(
                    FrameValue::new(false, false, 8 * 1024),
                    None,
                    None,
                ),
                width: 1920,
                height: 1080,
                rtp_timestamp: 2,
                frame_playout_deadline_at_ms: None,
                frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
                frame_unrecoverable_reason: None,
                assembled_at,
                h264: make_h264_inspection(false),
                payload: Bytes::from_static(b"d"),
            },
            Duration::from_millis(8),
            Duration::from_millis(30),
        );

        assert!(delta.target_playout_time < keyframe.target_playout_time);
    }

    #[test]
    fn recovery_window_promotes_delta_budget_to_supply_when_chain_cost_is_high() {
        let context = FrameBudgetContext::for_transport(
            FrameValue::new(false, false, 8 * 1024),
            true,
            Some(140.0),
            Some(1_018.0),
            Some(1_040.0),
            false,
            FrameBudgetWindowSource::Recovery,
        );

        assert_eq!(
            context.recovery_phase,
            FrameBudgetRecoveryPhase::AwaitingKeyframe
        );
        assert_eq!(context.link_value, FrameBudgetLinkValue::Supply);
        assert_eq!(context.failure_cost, FrameBudgetFailureCost::ChainBroken);
        assert!(context.prefers_chain_broken());
    }
}
