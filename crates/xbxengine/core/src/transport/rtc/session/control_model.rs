//! Session 层控制面最小词汇（RFC：`Stage` / `FaultDomain` / `CostCeiling`）的映射表。
//! 不引入新阈值；`coordinator` 内 `RecoverySignalDomain` 由本模块 `SessionFaultDomain` 映射得到。
//!
//! **契约**（详见仓库根目录 `docs/rfcs/2026-04-12-rtc-control-plane-eight-point-cleanup.md`）：
//! - 故障域分类的唯一入口：`resolve_session_fault_domain` / `resolve_session_fault_domain_from_owner_recovery_reason`。
//! - `RecoverySignalDomain` 不得独立演化语义，仅作 coordinator 内部桶。
//! - `CostCeiling` 动作梯子：`Absorb` → `LocalRecover` → `TransportRecover`（见 `SessionCostCeiling` / `session_cost_ceiling_for_recovery_action`）。

use crate::transport::rtc::policy::video_scheduling_owner::{
    OwnerRecoveryReason, VideoSchedulingOwnerState,
};
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};

/// 对应 RFC `FaultDomain`。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum SessionFaultDomain {
    Transport,
    ReferenceChain,
    DecodePipeline,
    DisplaySupply,
}

/// 对应 RFC `CostCeiling`。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum SessionCostCeiling {
    Absorb,
    LocalRecover,
    TransportRecover,
}

/// 对应 RFC `Stage`（与 `VideoSchedulingOwnerState` 粗映射，供诊断对齐）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum SessionRecoveryStage {
    Bootstrap,
    RecoveringToStable,
    Stable,
}

impl SessionRecoveryStage {
    pub(crate) fn as_rfc_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "Bootstrap",
            Self::RecoveringToStable => "RecoveringToStable",
            Self::Stable => "Stable",
        }
    }
}

impl SessionCostCeiling {
    pub(crate) fn as_rfc_str(self) -> &'static str {
        match self {
            Self::Absorb => "Absorb",
            Self::LocalRecover => "LocalRecover",
            Self::TransportRecover => "TransportRecover",
        }
    }
}

impl SessionFaultDomain {
    pub(crate) fn as_rfc_str(self) -> &'static str {
        match self {
            Self::Transport => "Transport",
            Self::ReferenceChain => "ReferenceChain",
            Self::DecodePipeline => "DecodePipeline",
            Self::DisplaySupply => "DisplaySupply",
        }
    }
}

/// owner 侧恢复意图 → coordinator 同源 `VideoEscalationReason`（与 `session::policy` 构造 `RecoveryOwnerSignal` 一致）。
pub(crate) fn owner_recovery_reason_to_media_escalation_reason(
    reason: OwnerRecoveryReason,
) -> Option<VideoEscalationReason> {
    match reason {
        OwnerRecoveryReason::TransportAwaitRecoveryKeyframe => {
            Some(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        }
        // RFC: decode 后显示域信号不得直接驱动媒体恢复动作。
        OwnerRecoveryReason::DisplaySupplyCritical
        | OwnerRecoveryReason::DisplaySupplyDegraded
        | OwnerRecoveryReason::HostPresentStalled => None,
    }
}

pub(crate) fn resolve_session_fault_domain_from_owner_recovery_reason(
    reason: OwnerRecoveryReason,
) -> SessionFaultDomain {
    match reason {
        OwnerRecoveryReason::TransportAwaitRecoveryKeyframe => SessionFaultDomain::ReferenceChain,
        OwnerRecoveryReason::DisplaySupplyCritical
        | OwnerRecoveryReason::DisplaySupplyDegraded
        | OwnerRecoveryReason::HostPresentStalled => SessionFaultDomain::DisplaySupply,
    }
}

pub(crate) fn resolve_session_fault_domain(reason: VideoEscalationReason) -> SessionFaultDomain {
    match reason {
        VideoEscalationReason::LifecycleRecovering
        | VideoEscalationReason::TransportExpiredDeadline
        | VideoEscalationReason::TransportSevereDeadline
        | VideoEscalationReason::TransportRecoveredLate
        | VideoEscalationReason::TransportSampleLoss => SessionFaultDomain::Transport,
        VideoEscalationReason::WaitKeyframe
        | VideoEscalationReason::TransportAwaitRecoveryKeyframe => {
            SessionFaultDomain::ReferenceChain
        }
        VideoEscalationReason::Reconfigure | VideoEscalationReason::DecoderBackendFailure => {
            SessionFaultDomain::DecodePipeline
        }
        VideoEscalationReason::DisplaySupplyCritical
        | VideoEscalationReason::AdapterIdleTimeout
        | VideoEscalationReason::AdapterThinStream => SessionFaultDomain::DisplaySupply,
    }
}

pub(crate) fn resolve_session_recovery_stage(
    state: VideoSchedulingOwnerState,
) -> SessionRecoveryStage {
    match state {
        VideoSchedulingOwnerState::SeekingAnchor | VideoSchedulingOwnerState::Priming => {
            SessionRecoveryStage::Bootstrap
        }
        VideoSchedulingOwnerState::RebuildingSupply
        | VideoSchedulingOwnerState::DegradedServing
        | VideoSchedulingOwnerState::SupplyStarved => SessionRecoveryStage::RecoveringToStable,
        VideoSchedulingOwnerState::StableServing => SessionRecoveryStage::Stable,
    }
}

pub(crate) fn session_cost_ceiling_for_recovery_action(
    action: RecoveryAction,
) -> SessionCostCeiling {
    match action {
        RecoveryAction::RequestReconnectCandidate => SessionCostCeiling::TransportRecover,
        RecoveryAction::RequestPli
        | RecoveryAction::RequestFir
        | RecoveryAction::RequestDecoderReset => SessionCostCeiling::LocalRecover,
        RecoveryAction::WaitForBurst
        | RecoveryAction::WaitForDecoderResetBurst
        | RecoveryAction::CooldownSuppressed
        | RecoveryAction::CoalescedKeyframeInFlight
        | RecoveryAction::CoalescedDecoderResetInFlight
        | RecoveryAction::StartupGraceSuppressed => SessionCostCeiling::Absorb,
    }
}

pub(crate) fn decode_or_display_fault_requires_transport_evidence(
    domain: SessionFaultDomain,
    ceiling: SessionCostCeiling,
) -> bool {
    matches!(
        domain,
        SessionFaultDomain::DecodePipeline | SessionFaultDomain::DisplaySupply
    ) && ceiling == SessionCostCeiling::TransportRecover
}
