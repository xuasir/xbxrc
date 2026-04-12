//! 昂贵恢复门控（reconnect 等）：在 `TransportAwait` 等路径上要求硬证据与本地无进展。
//! RFC：域语义以 `session::control_model` 与 `coordinator` 注释中的 FaultDomain 对照为准。

use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::scheduling::TwccWarmupState;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_observed_at_ms, has_current_transport_await_issue_from_observation,
};
use crate::transport::rtc::recovery::coordinator::{
    RecoveryCoordinator, RecoveryCoordinatorProposal,
};
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::has_fresh_media_output;
use crate::transport::rtc::session::control_model::{
    decode_or_display_fault_requires_transport_evidence, resolve_session_fault_domain,
    SessionCostCeiling,
};
use crate::XbxEngineMediaRuntimeStats;

const CONTROL_REPLAY_BACKLOG_HOLD_MS: f64 = 1_200.0;

pub(crate) struct ReconnectGateResolution {
    pub(crate) detail: Option<String>,
}

pub(crate) struct ExpensiveRecoveryGate<'a> {
    runtime_stats: &'a Mutex<XbxEngineMediaRuntimeStats>,
    is_cloud_gaming_profile: bool,
    reconnect_success_edge_at_last_grant: Option<f64>,
    last_successful_media_edge_at_ms: Option<f64>,
    reconnect_grants_without_success_edge: u8,
}

impl<'a> ExpensiveRecoveryGate<'a> {
    pub(crate) fn new(
        runtime_stats: &'a Mutex<XbxEngineMediaRuntimeStats>,
        is_cloud_gaming_profile: bool,
        reconnect_success_edge_at_last_grant: Option<f64>,
        last_successful_media_edge_at_ms: Option<f64>,
        reconnect_grants_without_success_edge: u8,
    ) -> Self {
        Self {
            runtime_stats,
            is_cloud_gaming_profile,
            reconnect_success_edge_at_last_grant,
            last_successful_media_edge_at_ms,
            reconnect_grants_without_success_edge,
        }
    }

    pub(crate) fn should_hold_media_reconnect_during_twcc_warmup(
        &self,
        reason: VideoEscalationReason,
        twcc_warmup_state: TwccWarmupState,
        action: RecoveryAction,
    ) -> bool {
        if !self.is_cloud_gaming_profile || !twcc_warmup_state.blocks_bwe_updates() {
            return false;
        }
        if action != RecoveryAction::RequestReconnectCandidate {
            return false;
        }
        reason != VideoEscalationReason::LifecycleRecovering
    }

    pub(crate) fn apply_to_proposal(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: &mut RecoveryCoordinatorProposal,
        observed_at_ms: f64,
        twcc_warmup_state: TwccWarmupState,
        block_lifecycle_reconnect_candidate: bool,
    ) -> ReconnectGateResolution {
        if proposal.decision.action != RecoveryAction::RequestReconnectCandidate {
            return ReconnectGateResolution { detail: None };
        }
        if block_lifecycle_reconnect_candidate {
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
            return ReconnectGateResolution {
                detail: Some("reconnectBlocked:lifecycleOverlap".to_string()),
            };
        }
        if self.should_hold_media_reconnect_during_twcc_warmup(
            proposal.signal.reason,
            twcc_warmup_state,
            proposal.decision.action,
        ) {
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
            return ReconnectGateResolution {
                detail: Some(format!(
                    "reconnectBlocked:twccWarmup:{}",
                    twcc_warmup_state.label()
                )),
            };
        }
        if let Some(block_reason) =
            self.media_reconnect_block_reason(snapshot, owner_state, proposal, observed_at_ms)
        {
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
            return ReconnectGateResolution {
                detail: Some(format!("reconnectBlocked:{block_reason}")),
            };
        }
        ReconnectGateResolution {
            detail: Some(resolve_reconnect_grant_detail(proposal)),
        }
    }

    /// RFC Appendix A：`DecodePipeline` / `DisplaySupply` 域不得在无进展且缺乏连接域硬证据时升入 `TransportRecover`。
    pub(crate) fn apply_rfc_decode_display_transport_ceiling(
        &self,
        snapshot: &TransportSnapshot,
        observed_at_ms: f64,
        recovery_no_progress_since_ms: Option<f64>,
        recovery_no_progress_fallback_ms: f64,
        local_self_healing_attempted: bool,
        media_progress_stalled: bool,
        has_connected_connectivity_failure_evidence: bool,
        proposal: &mut RecoveryCoordinatorProposal,
    ) {
        if proposal.decision.action != RecoveryAction::RequestReconnectCandidate {
            return;
        }
        let domain = resolve_session_fault_domain(proposal.signal.reason);
        if !decode_or_display_fault_requires_transport_evidence(domain, SessionCostCeiling::TransportRecover)
        {
            return;
        }
        let no_progress = recovery_no_progress_since_ms.is_some_and(|since| {
            (observed_at_ms - since).max(0.0) >= recovery_no_progress_fallback_ms
        }) || (local_self_healing_attempted && media_progress_stalled);
        let hard_evidence = RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
            stats.recovery_transport_await_unresolved == Some(true)
                || stats_has_unresolved_transport_await_issue(stats)
        })
        .unwrap_or(false)
            || RecoveryCoordinator::transport_await_has_hard_recovery_evidence(
                self.runtime_stats,
                proposal.budget_before.recovery_epoch,
                observed_at_ms,
            )
            || (snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Recovering
                && snapshot
                    .recovery
                    .latest_diagnosis_label
                    .as_deref()
                    == Some("rtcConnectionRecovering")
                && has_connected_connectivity_failure_evidence)
            || snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected;
        if !(no_progress && hard_evidence) {
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
        }
    }

    pub(crate) fn media_reconnect_block_reason(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: &RecoveryCoordinatorProposal,
        observed_at_ms: f64,
    ) -> Option<&'static str> {
        if proposal.decision.action != RecoveryAction::RequestReconnectCandidate {
            return None;
        }
        if matches!(
            proposal.signal.reason,
            VideoEscalationReason::LifecycleRecovering
                | VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSevereDeadline
                | VideoEscalationReason::TransportRecoveredLate
                | VideoEscalationReason::TransportSampleLoss
        ) {
            return None;
        }
        if proposal.signal.reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe {
            return Some("mediaGate:nonTransportAwait");
        }
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected {
            return Some("mediaGate:connectionNotConnected");
        }
        if !matches!(
            owner_state,
            VideoSchedulingOwnerState::RebuildingSupply | VideoSchedulingOwnerState::SupplyStarved
        ) {
            return Some("mediaGate:ownerNotRecoverySurface");
        }
        let recovery_epoch = proposal.budget_before.recovery_epoch;
        let control_replay_backlog_active =
            RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
                stats.control_pending_replay_action_count > 0
                    && stats.control_pending_replay_since_ms.is_some_and(|since| {
                        (observed_at_ms - since).max(0.0) <= CONTROL_REPLAY_BACKLOG_HOLD_MS
                    })
            })
            .unwrap_or(false);
        if control_replay_backlog_active {
            return Some("mediaGate:controlReplayBacklog");
        }
        if !RecoveryCoordinator::transport_await_has_hard_recovery_evidence(
            self.runtime_stats,
            recovery_epoch,
            observed_at_ms,
        ) {
            return Some("mediaGate:missingHardEvidence");
        }
        let awaiting_success_edge_after_grant = self
            .reconnect_success_edge_at_last_grant
            .zip(self.last_successful_media_edge_at_ms)
            .is_some_and(|(granted_edge, latest_edge)| granted_edge >= latest_edge)
            && self.reconnect_grants_without_success_edge > 0;
        if awaiting_success_edge_after_grant {
            return Some("mediaGate:awaitSuccessEdge");
        }
        let current_clean_anchor = RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
            stats
                .video_anchor_clean_epoch
                .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                && stats.video_anchor_clean_source_event.as_deref()
                    == Some("chain-clean-keyframe-submitted")
        })
        .unwrap_or(false);
        let local_progress_active = current_clean_anchor
            || RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
                has_fresh_media_output(stats, observed_at_ms)
            })
            .unwrap_or(false)
            || RecoveryCoordinator::transport_await_local_recovery_active(
                self.runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        if local_progress_active {
            return Some("mediaGate:localRecoveryActive");
        }
        None
    }
}

fn stats_has_unresolved_transport_await_issue(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let timeline = match stats.latest_video_timeline_observation.as_ref() {
        Some(timeline) => timeline,
        None => return false,
    };
    has_current_transport_await_issue_from_observation(
        timeline,
        current_clean_anchor_observed_at_ms(
            stats.video_anchor_clean_epoch,
            stats.video_anchor_clean_observed_at_ms,
            stats.video_anchor_clean_source_event.as_deref(),
            stats.transport_recovery_epoch,
        ),
    )
}

pub(crate) fn resolve_reconnect_grant_detail(proposal: &RecoveryCoordinatorProposal) -> String {
    let detail = if matches!(
        proposal.signal.reason,
        VideoEscalationReason::LifecycleRecovering
            | VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSevereDeadline
            | VideoEscalationReason::TransportRecoveredLate
            | VideoEscalationReason::TransportSampleLoss
    ) {
        "connectivityEvidence"
    } else if proposal.signal.reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe {
        "localRecoveryExhausted"
    } else {
        "policyPass"
    };
    format!("reconnectGranted:{detail}")
}
