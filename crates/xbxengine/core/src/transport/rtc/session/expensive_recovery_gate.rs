//! 昂贵恢复门控（reconnect 等）：在 `TransportAwait` 等路径上要求硬证据与本地无进展。
//! RFC：域语义以 `session::control_model` 与 `coordinator` 注释中的 FaultDomain 对照为准。

use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::scheduling::TwccWarmupState;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::contract::{
    has_current_clean_anchor_from_stats, has_current_transport_await_issue_from_stats,
    remote_picture_recovery_terminal_active_from_stats,
};
use crate::transport::rtc::recovery::coordinator::{CoordinatorProposal, RecoveryCoordinator};
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::has_fresh_media_output;
use crate::transport::rtc::session::control_model::{
    decode_or_display_fault_requires_transport_evidence, resolve_session_fault_domain,
    SessionCostCeiling,
};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineTwccObservationQuality};

const CONTROL_REPLAY_BACKLOG_HOLD_MS: f64 = 1_200.0;
const CONNECTED_HEALTHY_TRANSPORT_SIGNAL_FRESH_MS: f64 = 3_000.0;
const CONNECTED_HEALTHY_TWCC_MIN_DELIVERY_RATIO: f64 = 0.95;
const CONNECTED_HEALTHY_TRANSPORT_MAX_LOSS_RATIO: f64 = 0.05;
const CONNECTED_HEALTHY_TRANSPORT_MAX_RTT_MS: f64 = 500.0;
const CONNECTED_TRANSPORT_AWAIT_RECENT_SUCCESS_EDGE_MS: f64 = 4_000.0;
const LIFECYCLE_LIVENESS_REASON_LABEL: &str = "livenessNoProgressTimeout";
const ICE_DIRECT_NO_RESPONSE_PROBE_FRESH_MS: f64 = 2_500.0;
const ICE_DIRECT_NO_RESPONSE_MIN_REQUESTS: u64 = 8;

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
        proposal: &mut CoordinatorProposal,
        owner_signal: &crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal,
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
            owner_signal.reason,
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
        if let Some(block_reason) = self.reconnect_block_reason(
            snapshot,
            owner_state,
            proposal,
            owner_signal,
            observed_at_ms,
        ) {
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
            return ReconnectGateResolution {
                detail: Some(format!("reconnectBlocked:{block_reason}")),
            };
        }
        ReconnectGateResolution {
            detail: Some(resolve_reconnect_grant_detail(proposal, owner_signal)),
        }
    }

    fn reconnect_block_reason(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: &CoordinatorProposal,
        owner_signal: &crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal,
        observed_at_ms: f64,
    ) -> Option<&'static str> {
        match owner_signal.reason {
            VideoEscalationReason::LifecycleRecovering => self
                .lifecycle_liveness_reconnect_block_reason(
                    snapshot,
                    proposal,
                    owner_signal,
                    observed_at_ms,
                ),
            VideoEscalationReason::TransportLowValueDeadline
            | VideoEscalationReason::TransportRepairableDeadline => None,
            VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSevereDeadline
            | VideoEscalationReason::TransportRecoveredLate
            | VideoEscalationReason::TransportSampleLoss => {
                self.transport_deadline_reconnect_block_reason(snapshot, proposal, observed_at_ms)
            }
            _ => self.media_reconnect_block_reason(
                snapshot,
                owner_state,
                proposal,
                owner_signal,
                observed_at_ms,
            ),
        }
    }

    fn lifecycle_liveness_reconnect_block_reason(
        &self,
        snapshot: &TransportSnapshot,
        proposal: &CoordinatorProposal,
        owner_signal: &crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal,
        observed_at_ms: f64,
    ) -> Option<&'static str> {
        if proposal.decision.action != RecoveryAction::RequestReconnectCandidate {
            return None;
        }
        if owner_signal.reason_label != LIFECYCLE_LIVENESS_REASON_LABEL {
            return None;
        }
        if fresh_direct_ice_no_response_evidence(snapshot, observed_at_ms) {
            return None;
        }
        let (
            current_clean_anchor,
            remote_terminal_active,
            has_fresh_media_output,
            twcc_transport_healthy,
        ) = RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
            (
                has_current_clean_anchor_from_stats(stats),
                remote_picture_recovery_terminal_active_from_stats(stats),
                has_fresh_media_output(stats, observed_at_ms),
                twcc_transport_appears_healthy(stats, observed_at_ms),
            )
        })
        .unwrap_or((false, false, false, false));
        if remote_terminal_active {
            return None;
        }
        let awaiting_success_edge_after_grant = self
            .reconnect_success_edge_at_last_grant
            .zip(self.last_successful_media_edge_at_ms)
            .is_some_and(|(granted_edge, latest_edge)| granted_edge >= latest_edge)
            && self.reconnect_grants_without_success_edge > 0;
        if awaiting_success_edge_after_grant {
            return Some("lifecycleGate:awaitSuccessEdge");
        }
        if self.last_successful_media_edge_at_ms.is_some()
            && self.reconnect_grants_without_success_edge > 0
            && transport_rebuild_in_flight_without_direct_failure(snapshot, observed_at_ms)
        {
            return Some("lifecycleGate:transportRebuildInFlightNoProgress");
        }
        let control_replay_backlog_active =
            RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
                stats.control_pending_replay_action_count > 0
                    && stats.control_pending_replay_since_ms.is_some_and(|since| {
                        (observed_at_ms - since).max(0.0) <= CONTROL_REPLAY_BACKLOG_HOLD_MS
                    })
            })
            .unwrap_or(false);
        if control_replay_backlog_active {
            return Some("lifecycleGate:controlReplayBacklog");
        }
        let recent_success_edge = self
            .last_successful_media_edge_at_ms
            .is_some_and(|edge_at_ms| {
                (observed_at_ms - edge_at_ms).max(0.0)
                    <= CONNECTED_TRANSPORT_AWAIT_RECENT_SUCCESS_EDGE_MS
            });
        let recovery_or_display_already_serviceable =
            current_clean_anchor || has_fresh_media_output || recent_success_edge;
        if recovery_or_display_already_serviceable
            && (connected_transport_appears_healthy(snapshot, observed_at_ms)
                || twcc_transport_healthy)
        {
            return Some("lifecycleGate:connectedHealthyNoProgress");
        }
        let local_recovery_active = RecoveryCoordinator::transport_await_local_recovery_active(
            self.runtime_stats,
            proposal.budget_before.recovery_epoch,
            observed_at_ms,
        );
        if current_clean_anchor || has_fresh_media_output || local_recovery_active {
            return Some("lifecycleGate:localRecoveryActive");
        }
        if snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Connected
            && !connected_connectivity_failure_evidence(snapshot, observed_at_ms)
        {
            return Some("lifecycleGate:missingConnectivityEvidence");
        }
        None
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
        proposal: &mut CoordinatorProposal,
        owner_signal: &crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal,
    ) {
        if proposal.decision.action != RecoveryAction::RequestReconnectCandidate {
            return;
        }
        let domain = resolve_session_fault_domain(owner_signal.reason);
        if !decode_or_display_fault_requires_transport_evidence(
            domain,
            SessionCostCeiling::TransportRecover,
        ) {
            return;
        }
        let no_progress = recovery_no_progress_since_ms.is_some_and(|since| {
            (observed_at_ms - since).max(0.0) >= recovery_no_progress_fallback_ms
        }) || (local_self_healing_attempted && media_progress_stalled);
        let hard_evidence = RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
            remote_picture_recovery_terminal_active_from_stats(stats)
                || stats.recovery_transport_await_unresolved == Some(true)
                || stats_has_unresolved_transport_await_issue(stats)
        })
        .unwrap_or(false)
            || RecoveryCoordinator::transport_await_has_hard_recovery_evidence(
                self.runtime_stats,
                proposal.budget_before.recovery_epoch,
                observed_at_ms,
            )
            || (snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Recovering
                && RuntimeStatsSink::read_shared(&self.runtime_stats, |stats| {
                    crate::transport::rtc::recovery::escalation_label::effective_recovery_control_label(
                        snapshot.recovery.latest_diagnosis_label.as_deref(),
                        stats,
                    )
                })
                .flatten()
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
        proposal: &CoordinatorProposal,
        owner_signal: &crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal,
        observed_at_ms: f64,
    ) -> Option<&'static str> {
        if proposal.decision.action != RecoveryAction::RequestReconnectCandidate {
            return None;
        }
        if owner_signal.reason == VideoEscalationReason::LifecycleRecovering {
            return None;
        }
        if owner_signal.reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe {
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
        let (
            current_clean_anchor,
            remote_terminal_active,
            has_fresh_media_output,
            twcc_transport_healthy,
        ) = RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
            (
                has_current_clean_anchor_from_stats(stats),
                remote_picture_recovery_terminal_active_from_stats(stats),
                has_fresh_media_output(stats, observed_at_ms),
                twcc_transport_appears_healthy(stats, observed_at_ms),
            )
        })
        .unwrap_or((false, false, false, false));
        let recent_success_edge = self
            .last_successful_media_edge_at_ms
            .is_some_and(|edge_at_ms| {
                (observed_at_ms - edge_at_ms).max(0.0)
                    <= CONNECTED_TRANSPORT_AWAIT_RECENT_SUCCESS_EDGE_MS
            });
        let recovery_or_display_already_serviceable =
            current_clean_anchor || has_fresh_media_output || recent_success_edge;
        if !remote_terminal_active
            && recovery_or_display_already_serviceable
            && (connected_transport_appears_healthy(snapshot, observed_at_ms)
                || twcc_transport_healthy)
        {
            return Some("mediaGate:connectedHealthyTransportAwait");
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
        if self.last_successful_media_edge_at_ms.is_some()
            && self.reconnect_grants_without_success_edge > 0
            && transport_rebuild_in_flight_without_direct_failure(snapshot, observed_at_ms)
        {
            return Some("mediaGate:transportRebuildInFlightNoProgress");
        }
        let local_recovery_active = RecoveryCoordinator::transport_await_local_recovery_active(
            self.runtime_stats,
            recovery_epoch,
            observed_at_ms,
        );
        let local_progress_active = !remote_terminal_active
            && (current_clean_anchor || has_fresh_media_output || local_recovery_active);
        if local_progress_active {
            return Some("mediaGate:localRecoveryActive");
        }
        None
    }

    fn transport_deadline_reconnect_block_reason(
        &self,
        snapshot: &TransportSnapshot,
        proposal: &CoordinatorProposal,
        observed_at_ms: f64,
    ) -> Option<&'static str> {
        if proposal.decision.action != RecoveryAction::RequestReconnectCandidate {
            return None;
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
            return Some("transportGate:controlReplayBacklog");
        }
        let awaiting_success_edge_after_grant = self
            .reconnect_success_edge_at_last_grant
            .zip(self.last_successful_media_edge_at_ms)
            .is_some_and(|(granted_edge, latest_edge)| granted_edge >= latest_edge)
            && self.reconnect_grants_without_success_edge > 0;
        if awaiting_success_edge_after_grant {
            return Some("transportGate:awaitSuccessEdge");
        }
        let current_clean_anchor =
            RuntimeStatsSink::read_shared(self.runtime_stats, has_current_clean_anchor_from_stats)
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
            return Some("transportGate:localRecoveryActive");
        }
        let unresolved_transport_await =
            RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
                stats.recovery_transport_await_unresolved == Some(true)
                    || stats_has_unresolved_transport_await_issue(stats)
            })
            .unwrap_or(false);
        let transport_await_hard_failure =
            RecoveryCoordinator::transport_await_has_hard_recovery_evidence(
                self.runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        if unresolved_transport_await && !transport_await_hard_failure {
            return Some("transportGate:awaitingRecoveryChain");
        }
        if snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Connected
            && !connected_connectivity_failure_evidence(snapshot, observed_at_ms)
        {
            return Some("transportGate:missingConnectivityEvidence");
        }
        None
    }
}

fn connected_transport_appears_healthy(snapshot: &TransportSnapshot, observed_at_ms: f64) -> bool {
    if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected {
        return false;
    }
    let transport_signal_fresh = snapshot.connection.last_observed_at_ms.is_some_and(|last| {
        (observed_at_ms - last).max(0.0) <= CONNECTED_HEALTHY_TRANSPORT_SIGNAL_FRESH_MS
    });
    if !transport_signal_fresh {
        return false;
    }
    let has_channel = snapshot.connection.control_channel_open
        || snapshot.connection.message_channel_open
        || snapshot.connection.input_channel_open
        || snapshot.connection.chat_channel_open;
    let has_transport_path = snapshot.connection.latest_transport_path.is_some();
    let has_selected_pair = snapshot.connection.ice_has_selected_or_nominated_pair
        || snapshot.connection.ice_nominated_pair_count > 0
        || snapshot.connection.ice_succeeded_pair_count > 0;
    let has_rtt = snapshot.connection.latest_rtt_ms.is_some();
    if !(has_channel || has_transport_path || has_selected_pair || has_rtt) {
        return false;
    }
    let rtt_healthy = snapshot
        .connection
        .latest_rtt_ms
        .is_none_or(|rtt_ms| rtt_ms <= CONNECTED_HEALTHY_TRANSPORT_MAX_RTT_MS);
    let loss_healthy = snapshot
        .connection
        .latest_loss_ratio_1s
        .is_none_or(|loss| loss <= CONNECTED_HEALTHY_TRANSPORT_MAX_LOSS_RATIO);
    rtt_healthy && loss_healthy && !snapshot.connection.ice_direct_checks_without_response
}

fn twcc_transport_appears_healthy(stats: &XbxEngineMediaRuntimeStats, observed_at_ms: f64) -> bool {
    if stats.transport_state != xbxengine_protocol::XbxEngineTransportStateDto::Connected {
        return false;
    }
    let Some(twcc) = stats.latest_video_twcc_observation.as_ref() else {
        return false;
    };
    let twcc_fresh = (observed_at_ms - twcc.observed_at_ms).max(0.0)
        <= CONNECTED_HEALTHY_TRANSPORT_SIGNAL_FRESH_MS;
    twcc_fresh
        && twcc.is_local_feedback()
        && twcc.twcc_sample_valid
        && twcc.quality == XbxEngineTwccObservationQuality::Stable
        && twcc.delivery_ratio >= CONNECTED_HEALTHY_TWCC_MIN_DELIVERY_RATIO
        && twcc.packet_loss_ratio <= CONNECTED_HEALTHY_TRANSPORT_MAX_LOSS_RATIO
        && !stats
            .latest_ice_connectivity_probe
            .as_ref()
            .is_some_and(|probe| {
                probe.direct_checks_without_response
                    && (observed_at_ms - probe.observed_at_ms).max(0.0)
                        <= CONNECTED_HEALTHY_TRANSPORT_SIGNAL_FRESH_MS
            })
}

fn fresh_direct_ice_no_response_evidence(
    snapshot: &TransportSnapshot,
    observed_at_ms: f64,
) -> bool {
    if !matches!(
        snapshot.connection.lifecycle_state,
        ConnectionLifecycleStateFact::Connecting | ConnectionLifecycleStateFact::Recovering
    ) {
        return false;
    }
    if !snapshot.connection.ice_direct_checks_without_response {
        return false;
    }
    if snapshot.connection.ice_has_selected_or_nominated_pair {
        return false;
    }
    if snapshot.connection.ice_max_requests_sent < ICE_DIRECT_NO_RESPONSE_MIN_REQUESTS {
        return false;
    }
    if snapshot.connection.ice_responses_received_total != 0 {
        return false;
    }
    snapshot
        .connection
        .ice_probe_observed_at_ms
        .is_some_and(|at_ms| {
            (observed_at_ms - at_ms).max(0.0) <= ICE_DIRECT_NO_RESPONSE_PROBE_FRESH_MS
        })
}

fn transport_rebuild_in_flight_without_direct_failure(
    snapshot: &TransportSnapshot,
    observed_at_ms: f64,
) -> bool {
    snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Connecting
        && !fresh_direct_ice_no_response_evidence(snapshot, observed_at_ms)
}

fn stats_has_unresolved_transport_await_issue(stats: &XbxEngineMediaRuntimeStats) -> bool {
    has_current_transport_await_issue_from_stats(stats)
}

fn connected_connectivity_failure_evidence(
    snapshot: &TransportSnapshot,
    observed_at_ms: f64,
) -> bool {
    let has_data_channel = snapshot.connection.control_channel_open
        || snapshot.connection.message_channel_open
        || snapshot.connection.input_channel_open
        || snapshot.connection.chat_channel_open;
    let has_transport_signal = snapshot.connection.latest_transport_path.is_some()
        || snapshot.connection.latest_rtt_ms.is_some();
    let connection_signal_stale = snapshot
        .connection
        .last_observed_at_ms
        .is_none_or(|last| (observed_at_ms - last).max(0.0) >= 2_000.0);
    !has_data_channel && !has_transport_signal && connection_signal_stale
}

pub(crate) fn resolve_reconnect_grant_detail(
    _proposal: &CoordinatorProposal,
    owner_signal: &crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal,
) -> String {
    let detail = if matches!(
        owner_signal.reason,
        VideoEscalationReason::LifecycleRecovering
            | VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSevereDeadline
            | VideoEscalationReason::TransportRecoveredLate
            | VideoEscalationReason::TransportSampleLoss
    ) {
        "connectivityEvidence"
    } else if matches!(
        owner_signal.reason,
        VideoEscalationReason::TransportLowValueDeadline
            | VideoEscalationReason::TransportRepairableDeadline
    ) {
        "localTransportRepair"
    } else if owner_signal.reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe {
        "localRecoveryExhausted"
    } else {
        "policyPass"
    };
    format!("reconnectGranted:{detail}")
}
