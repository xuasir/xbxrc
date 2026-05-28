//! 统一的恢复观察层
//!
//! 替代原有的Signal + Diagnosis两层架构，直接从事件映射到恢复严重性。

use super::escalation::VideoEscalationReason;
use crate::transport::rtc::session::facts::GapSeverity;

/// 恢复观察严重性
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum RecoverySeverity {
    /// 轻微问题（可能自愈）
    Minor,
    /// 丢包（需要NACK）
    PacketLoss,
    /// 链断裂（需要IDR）
    ChainBroken,
    /// 解码问题（需要decoder reset）
    DecoderIssue,
    /// 传输问题（需要reconnect）
    TransportIssue,
}

/// 统一的恢复观察
#[derive(Clone, Debug)]
pub(crate) struct RecoveryObservation {
    /// 严重性
    pub(crate) severity: RecoverySeverity,
    /// 原因标签
    pub(crate) reason_label: String,
    /// Gap严重性（如果适用）
    pub(crate) gap_severity: Option<GapSeverity>,
    /// Repairability评分（如果适用）
    pub(crate) repairability: Option<f64>,
}

impl RecoveryObservation {
    /// 从VideoEscalationReason创建观察
    pub(crate) fn from_reason(
        reason: VideoEscalationReason,
        reason_label: String,
        _observed_at_ms: f64,
    ) -> Self {
        let severity = Self::classify_severity(reason);

        Self {
            severity,
            reason_label,
            gap_severity: None,
            repairability: None,
        }
    }

    /// 带gap严重性的观察
    pub(crate) fn with_gap_severity(mut self, gap_severity: GapSeverity) -> Self {
        self.gap_severity = Some(gap_severity);
        // 根据gap严重性调整severity
        self.severity = self.severity.max(Self::severity_from_gap(gap_severity));
        self
    }

    /// 带repairability评分的观察
    pub(crate) fn with_repairability(mut self, repairability: f64) -> Self {
        self.repairability = Some(repairability);
        self
    }

    /// 分类严重性
    fn classify_severity(reason: VideoEscalationReason) -> RecoverySeverity {
        match reason {
            // 传输问题 → 需要reconnect
            VideoEscalationReason::LifecycleRecovering => RecoverySeverity::TransportIssue,

            // 解码问题 → 需要decoder reset
            VideoEscalationReason::Reconfigure | VideoEscalationReason::DecoderBackendFailure => {
                RecoverySeverity::DecoderIssue
            }

            // 链断裂 → 需要IDR
            VideoEscalationReason::WaitKeyframe
            | VideoEscalationReason::TransportAwaitRecoveryKeyframe => {
                RecoverySeverity::ChainBroken
            }

            // RFC: decode 后显示域问题只做本地吸收/自愈，不得直接驱动媒体恢复动作。
            VideoEscalationReason::DisplaySupplyCritical
            | VideoEscalationReason::LocalSupplySuspect
            | VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::AdapterThinStream
            | VideoEscalationReason::TransportLowValueDeadline
            | VideoEscalationReason::TransportRepairableDeadline => RecoverySeverity::Minor,

            // 丢包 → 需要NACK或IDR（根据repairability决定）
            VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSampleLoss => RecoverySeverity::PacketLoss,

            // 严重deadline → 可能需要IDR
            VideoEscalationReason::TransportSevereDeadline => RecoverySeverity::ChainBroken,

            // 恢复延迟 → 轻微问题
            VideoEscalationReason::TransportRecoveredLate => RecoverySeverity::Minor,
        }
    }

    /// 从gap严重性推导severity
    fn severity_from_gap(gap_severity: GapSeverity) -> RecoverySeverity {
        match gap_severity {
            GapSeverity::LowValueGap | GapSeverity::RepairableGap => RecoverySeverity::Minor,
            GapSeverity::ReferenceGap => RecoverySeverity::PacketLoss,
            GapSeverity::AnchorGap => RecoverySeverity::ChainBroken,
            GapSeverity::ChainBroken => RecoverySeverity::ChainBroken,
            GapSeverity::RecoveryBlocked => RecoverySeverity::ChainBroken,
        }
    }

    /// 检查是否需要升级到IDR（基于repairability）
    pub(crate) fn should_escalate_to_idr(&self, repairability_threshold: f64) -> bool {
        match self.severity {
            RecoverySeverity::PacketLoss => {
                // 如果repairability低于阈值，升级到IDR
                if let Some(rep) = self.repairability {
                    rep <= repairability_threshold
                } else {
                    false
                }
            }
            RecoverySeverity::ChainBroken
            | RecoverySeverity::DecoderIssue
            | RecoverySeverity::TransportIssue => true,
            _ => false,
        }
    }

    /// 检查是否需要decoder reset
    pub(crate) fn requires_decoder_reset(&self) -> bool {
        matches!(self.severity, RecoverySeverity::DecoderIssue)
    }

    /// 检查是否需要reconnect
    pub(crate) fn requires_reconnect(&self) -> bool {
        matches!(self.severity, RecoverySeverity::TransportIssue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_classification() {
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1000.0,
        );
        assert_eq!(obs.severity, RecoverySeverity::ChainBroken);

        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::TransportExpiredDeadline,
            "transportExpiredDeadline".to_string(),
            1000.0,
        );
        assert_eq!(obs.severity, RecoverySeverity::PacketLoss);

        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::DecoderBackendFailure,
            "decoderBackendFailure".to_string(),
            1000.0,
        );
        assert_eq!(obs.severity, RecoverySeverity::DecoderIssue);

        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::LifecycleRecovering,
            "rtcConnectionRecovering".to_string(),
            1000.0,
        );
        assert_eq!(obs.severity, RecoverySeverity::TransportIssue);
    }

    #[test]
    fn test_gap_severity_adjustment() {
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::TransportExpiredDeadline,
            "transportExpiredDeadline".to_string(),
            1000.0,
        )
        .with_gap_severity(GapSeverity::ChainBroken);

        assert_eq!(obs.severity, RecoverySeverity::ChainBroken);
    }

    #[test]
    fn test_repairability_escalation() {
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::TransportExpiredDeadline,
            "transportExpiredDeadline".to_string(),
            1000.0,
        )
        .with_repairability(0.3);

        // repairability 0.3 < 0.45 → 应该升级到IDR
        assert!(obs.should_escalate_to_idr(0.45));

        let obs = obs.with_repairability(0.6);
        // repairability 0.6 > 0.45 → 不应该升级
        assert!(!obs.should_escalate_to_idr(0.45));
    }

    #[test]
    fn test_recovery_requirements() {
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::DecoderBackendFailure,
            "decoderBackendFailure".to_string(),
            1000.0,
        );
        assert!(obs.requires_decoder_reset());
        assert!(!obs.requires_reconnect());

        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::LifecycleRecovering,
            "rtcConnectionRecovering".to_string(),
            1000.0,
        );
        assert!(!obs.requires_decoder_reset());
        assert!(obs.requires_reconnect());
    }
}
