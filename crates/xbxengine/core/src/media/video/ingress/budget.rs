//! 帧级预算：`FrameBudgetContext`、链路价值与 RTT slack。
//! RFC：帧级价值不上升到 `session::policy`；禁止在此直接下发 transport 级昂贵恢复动作。

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
pub(crate) enum DynamicRepairValueTier {
    Anchor,
    Continuation,
    Supply,
    #[default]
    Disposable,
}

impl DynamicRepairValueTier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Anchor => "anchor",
            Self::Continuation => "continuation",
            Self::Supply => "supply",
            Self::Disposable => "disposable",
        }
    }
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
    pub(crate) fn decode_local_budget_ms(&self) -> u64 {
        match self.window_source {
            // recovery burst 只放宽时间预算，不授予长期队列特权。
            FrameBudgetWindowSource::Recovery => 96,
            FrameBudgetWindowSource::Reconfigure => 120,
            FrameBudgetWindowSource::Transport | FrameBudgetWindowSource::Playout => 48,
        }
    }

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
            Some("referenceChainUnrecoverable" | "awaitingRecoveryAnchor") => {
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
            Some("referenceChainUnrecoverable" | "awaitingRecoveryAnchor")
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

    /// 返回 link_value 对应的恢复价值分档标签，用于日志和观测。
    /// 注意：这是恢复价值分档，不是 H.264 媒体类型（媒体类型由 NAL inspection 决定）。
    ///
    /// 返回值：
    /// - "anchor": 锚点帧，恢复链路的关键节点
    /// - "supply": 供给帧，提供参考但非关键
    /// - "disposable": 可丢弃帧，丢失不影响恢复
    pub(crate) fn recovery_value_tier(&self) -> &'static str {
        match self.link_value {
            FrameBudgetLinkValue::Anchor => "anchor",
            FrameBudgetLinkValue::Supply => "supply",
            FrameBudgetLinkValue::Disposable => "disposable",
        }
    }

    pub(crate) fn dynamic_repair_value_tier(&self) -> DynamicRepairValueTier {
        match self.link_value {
            FrameBudgetLinkValue::Anchor => DynamicRepairValueTier::Anchor,
            FrameBudgetLinkValue::Supply => {
                if matches!(
                    self.recovery_phase,
                    FrameBudgetRecoveryPhase::AwaitingKeyframe
                        | FrameBudgetRecoveryPhase::Repairing
                ) {
                    DynamicRepairValueTier::Continuation
                } else {
                    DynamicRepairValueTier::Supply
                }
            }
            FrameBudgetLinkValue::Disposable => DynamicRepairValueTier::Disposable,
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

    pub(crate) fn retry_budget(&self, _value: FrameValue, _default_max_retry_count: u8) -> u8 {
        match (self.link_value, self.failure_cost, self.rtt_slack) {
            (_, FrameBudgetFailureCost::ChainBroken, FrameBudgetRttSlack::Exhausted) => 0,
            // NACK 统一采用单发策略。
            // deadline/maxAge 负责控制“还等不等它回来”，poll 不再触发二次发送。
            (FrameBudgetLinkValue::Anchor, _, _) => 0,
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
    let target_playout_instant =
        frame.first_packet_arrived_at.unwrap_or(frame.assembled_at) + playout_delay;
    frame.into_encoded_frame(target_playout_instant)
}

fn resolve_playout_delay(
    value: FrameValue,
    min_delay: Duration,
    max_delay: Duration,
    context: FrameBudgetContext,
) -> Duration {
    let anchor_or_chain_broken = matches!(context.link_value, FrameBudgetLinkValue::Anchor)
        || matches!(context.failure_cost, FrameBudgetFailureCost::ChainBroken);
    let mut ratio = context.deadline_budget_ratio_per_mille(value);
    if anchor_or_chain_broken {
        // 恢复关键帧在 RTT 已知且充裕时允许收紧 ratio，避免恒定顶格 max_delay。
        // Unknown 表示没有 RTT 信息，保守处理不干预；Exhausted 表示余量耗尽，
        // 同样不收紧（让 ratio 保持高位以保留最大等待窗口）。
        ratio = match context.rtt_slack {
            FrameBudgetRttSlack::Ample => ratio.min(700),
            FrameBudgetRttSlack::Tight => ratio.min(960),
            FrameBudgetRttSlack::Exhausted => ratio.min(1_100),
            FrameBudgetRttSlack::Unknown => ratio,
        };
    }
    let ratio = ratio as u128;
    let min_ms = min_delay.as_millis();
    let max_ms = max_delay.as_millis().max(min_ms);
    let spread_ms = max_ms.saturating_sub(min_ms);
    let mut scaled_ms = min_ms + (spread_ms * ratio / 1_000);
    if anchor_or_chain_broken {
        // 防抖偏置：在 ratio 已被 cap 后叠加一个小的固定增量，防止 ratio 恰好落在
        // cap 边界时被截断导致窗口过窄。Unknown 场景 ratio 未被 cap，偏置最大以保守兜底。
        let protection_bias_ms: u128 = match context.rtt_slack {
            FrameBudgetRttSlack::Ample => 2,
            FrameBudgetRttSlack::Tight => 4,
            FrameBudgetRttSlack::Exhausted => 5,
            FrameBudgetRttSlack::Unknown => 7,
        };
        scaled_ms = scaled_ms.saturating_add(protection_bias_ms).min(max_ms);
    }
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
                first_packet_sequence: None,
                frame_playout_deadline_at_ms: None,
                frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
                frame_unrecoverable_reason: None,
                assembled_at,
                first_packet_arrived_at: None,
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
                first_packet_sequence: None,
                frame_playout_deadline_at_ms: None,
                frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
                frame_unrecoverable_reason: None,
                assembled_at,
                first_packet_arrived_at: None,
                h264: make_h264_inspection(false),
                payload: Bytes::from_static(b"d"),
            },
            Duration::from_millis(8),
            Duration::from_millis(30),
        );

        assert!(delta.target_playout_instant < keyframe.target_playout_instant);
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

    #[test]
    fn recovery_window_reference_uses_single_shot_budget() {
        let context = FrameBudgetContext::for_transport(
            FrameValue::new(false, true, 48 * 1024),
            false,
            Some(90.0),
            Some(1_018.0),
            Some(1_040.0),
            false,
            FrameBudgetWindowSource::Recovery,
        );

        assert_eq!(context.link_value, FrameBudgetLinkValue::Supply);
        assert_eq!(context.failure_cost, FrameBudgetFailureCost::LocalDrop);
        assert_eq!(
            context.retry_budget(FrameValue::new(false, true, 48 * 1024), 3),
            0
        );
    }

    #[test]
    fn refresh_boost_supply_still_uses_single_shot_budget() {
        let context = FrameBudgetContext::for_transport(
            FrameValue::new(false, true, 48 * 1024),
            false,
            Some(90.0),
            Some(1_018.0),
            Some(1_060.0),
            false,
            FrameBudgetWindowSource::Recovery,
        );

        assert_eq!(context.link_value, FrameBudgetLinkValue::Supply);
        assert_eq!(context.failure_cost, FrameBudgetFailureCost::LocalDrop);
        assert_eq!(
            context.retry_budget(FrameValue::new(false, true, 48 * 1024), 3),
            0
        );
    }

    #[test]
    fn materialization_prefers_first_packet_arrived_at_when_present() {
        let assembled_at = Instant::now();
        let first_packet_arrived_at = assembled_at - Duration::from_millis(15);
        let encoded = materialize_ingress_frame(
            AssembledVideoFrame {
                codec: VideoCodec::H264,
                is_keyframe: false,
                config_changed: false,
                value: FrameValue::new(false, false, 8 * 1024),
                budget: FrameBudgetContext::for_transport(
                    FrameValue::new(false, false, 8 * 1024),
                    false,
                    Some(40.0),
                    Some(1_000.0),
                    Some(1_100.0),
                    false,
                    FrameBudgetWindowSource::Playout,
                ),
                width: 1920,
                height: 1080,
                rtp_timestamp: 3,
                first_packet_sequence: None,
                frame_playout_deadline_at_ms: None,
                frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
                frame_unrecoverable_reason: None,
                assembled_at,
                first_packet_arrived_at: Some(first_packet_arrived_at),
                h264: make_h264_inspection(false),
                payload: Bytes::from_static(b"d"),
            },
            Duration::from_millis(8),
            Duration::from_millis(30),
        );
        assert!(encoded.target_playout_instant < assembled_at + Duration::from_millis(25));
    }

    #[test]
    fn adaptive_anchor_delay_is_not_stuck_at_max_delay_under_ample_rtt() {
        let context = FrameBudgetContext::for_transport(
            FrameValue::new(true, true, 64 * 1024),
            false,
            Some(28.0),
            Some(1_000.0),
            Some(1_100.0),
            false,
            FrameBudgetWindowSource::Recovery,
        );
        let delay = super::resolve_playout_delay(
            FrameValue::new(true, true, 64 * 1024),
            Duration::from_millis(20),
            Duration::from_millis(30),
            context,
        );
        assert!(delay < Duration::from_millis(30));
    }
}
