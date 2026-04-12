use std::sync::{Arc, Mutex};

use crate::api::backend::{
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction, XbxEngineVideoBweObservation,
};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::executor::peer::{
    stage_reconnect_candidate, StageReconnectCandidateOutcome,
};
use crate::transport::rtc::facts::{
    CommandResultFact, CommandResultStatus, RecoveryEscalationFact, SessionCommand, TimerFact,
    TransportCommand, TransportFact,
};
use crate::transport::rtc::recovery::contract::{
    derive_gap_severity_with_episode_stall, frame_value_from_gap_severity,
    is_terminal_transport_await_deferred_episode, RecoveryEpisodeStage,
};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::RtcMediaService;
use crate::XbxEngineRuntimeError;

/// 恢复命令族的 in-flight 观察窗口：用于识别同族合并与升级，而不是回落到泛化 cooldown。
const RECOVERY_COMMAND_FAMILY_IN_FLIGHT_WINDOW_MS: f64 = 960.0;
/// 当关键帧已经解码但迟迟没有 clean anchor 提交时，不应长期占坑。
/// 这里用一个短窗口允许闭环推进；超窗后允许解锁并交给更强动作/抢占语义处理。
const KEYFRAME_DECODED_PENDING_COMMIT_HOLD_MS: f64 = 420.0;
/// 关键帧请求发出后长时间没有任何“推进边沿”（首包/响应/解码/clean anchor），应判定为 stalled，
/// 允许解锁占坑，避免同 family 无限压制导致的“假恢复”。
const KEYFRAME_NO_PROGRESS_STALL_MS: f64 = 900.0;
/// decoder reset 后如果迟迟没有任何输出恢复，也不应无限被视为 in-flight。
const DECODER_RESET_PROGRESS_HOLD_MS: f64 = 900.0;
/// reset 后看到明确的无效恢复响应（例如 NonIdrVcl），应立即解除 decoder-reset family gate。
const INVALID_KEYFRAME_RESPONSE_FRESH_MS: f64 = 1_500.0;
const RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_DECODER_RESET: &str =
    "familyInFlight:decoderResetInFlight";
const RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_CONTROL_PENDING: &str =
    "familyInFlight:controlChannelPending";
const RECOVERY_COMMAND_REASON_SAME_FAMILY_KEYFRAME_COALESCED: &str =
    "sameFamilyCoalesced:keyframeInFlight";
const RECOVERY_COMMAND_REASON_SAME_FAMILY_KEYFRAME_REFRESHED: &str =
    "sameFamilyRefreshed:keyframeEpisode";
const RECOVERY_COMMAND_REASON_SAME_FAMILY_DECODER_RESET_COALESCED: &str =
    "sameFamilyCoalesced:decoderResetInFlight";
const RECOVERY_COMMAND_REASON_SAME_FAMILY_TRANSPORT_STAGE_COALESCED: &str =
    "sameFamilyCoalesced:transportStageSuppressed";
const RECOVERY_COMMAND_SEMANTIC_FAMILY_UPGRADE_KEYFRAME_TO_DECODER_RESET: &str =
    "familyUpgrade:keyframeInFlight->decoderReset";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryCommandKind {
    RequestKeyframe,
    RequestDecoderReset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecoveryCommandFamilyDecision {
    Proceed,
    Refresh { detail: String },
    Defer { reason: String },
    Upgrade { detail: String },
}

#[derive(Clone, Debug, Default)]
struct RecoveryCommandDecisionSemantics {
    frame_value: Option<&'static str>,
    gap_severity: Option<&'static str>,
    recovery_episode_stage: Option<&'static str>,
    recovery_episode_progress_at_ms: Option<f64>,
    coalescing_mode: Option<&'static str>,
    unlock_reason: Option<&'static str>,
    preempt_reason: Option<&'static str>,
    recovery_primary_action: Option<&'static str>,
}

fn latest_decoder_reset_family_attempt_at_ms(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
    let action_attempt_ms =
        stats
            .latest_video_escalation_observation
            .as_ref()
            .and_then(|observation| {
                matches!(
                    observation.action.as_str(),
                    "requestDecoderReset"
                        | "requestKeyframe+decoderReset"
                        | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                )
                .then_some(observation.observed_at_ms)
            });
    match (stats.latest_video_decoder_reset_time_ms, action_attempt_ms) {
        (Some(reset_at_ms), Some(action_at_ms)) => Some(reset_at_ms.max(action_at_ms)),
        (Some(reset_at_ms), None) => Some(reset_at_ms),
        (None, Some(action_at_ms)) => Some(action_at_ms),
        (None, None) => None,
    }
}

fn has_post_decoder_reset_progress(stats: &XbxEngineMediaRuntimeStats, attempt_at_ms: f64) -> bool {
    stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| at_ms > attempt_at_ms)
        || stats
            .latest_video_host_present_time_ms
            .is_some_and(|at_ms| at_ms > attempt_at_ms)
        || stats
            .video_anchor_clean_observed_at_ms
            .is_some_and(|at_ms| at_ms > attempt_at_ms)
}

fn has_transport_await_invalid_keyframe_response_after_decoder_reset(
    stats: &XbxEngineMediaRuntimeStats,
    attempt_at_ms: f64,
    now_ms: f64,
) -> bool {
    let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
        return false;
    };
    if episode.request_reason.as_deref() != Some("transportAwaitRecoveryKeyframe") {
        return false;
    }
    let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() else {
        return false;
    };
    if inspection.observed_at_ms < attempt_at_ms
        || (now_ms - inspection.observed_at_ms).max(0.0) > INVALID_KEYFRAME_RESPONSE_FRESH_MS
    {
        return false;
    }
    // NonIdrVcl：reset 后只要 inspection 在 attempt 之后且仍新鲜，即可认定无效响应，
    // 不必等待 packet-seen/decoded episode 边沿（与 keyframe 占坑侧 invalid-bootstrap 语义对齐）。
    if !inspection.bootstrap_ready
        && inspection.bootstrap_reject_reason.as_deref() == Some("NonIdrVcl")
    {
        return true;
    }
    let packet_seen_without_decode = episode.status == "packet-seen"
        && episode.first_keyframe_decoded_at_ms.is_none()
        && episode
            .first_keyframe_packet_at_ms
            .is_some_and(|packet_at_ms| packet_at_ms >= attempt_at_ms);
    let decoded_without_clean_anchor = episode.status == "decoded"
        && episode
            .first_keyframe_decoded_at_ms
            .is_some_and(|decoded_at_ms| decoded_at_ms >= attempt_at_ms)
        && !matches!(
            stats.video_anchor_clean_epoch,
            Some(epoch) if epoch == stats.transport_recovery_epoch
        );
    if !(packet_seen_without_decode || decoded_without_clean_anchor) {
        return false;
    }
    !inspection.bootstrap_ready
        && matches!(
            inspection.bootstrap_reject_reason.as_deref(),
            Some(
                "bootstrapMissingSps"
                    | "bootstrapMissingPps"
                    | "inspectionRejectInvalidSliceHeader"
            )
        )
}

// 负责把 transport fact/command 和 connection/media 副作用桥接起来，
// 让 stack.rs 只保留编排入口。
pub(crate) struct RtcTransportSessionBridge<'a> {
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
    pending_runtime_recovery_action: &'a Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    connection: &'a Arc<Mutex<RtcConnectionService>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
    local_decoder_reset_handle:
        &'a Arc<Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>>,
    transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
    transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
}

impl<'a> RtcTransportSessionBridge<'a> {
    pub(crate) fn new(
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
        pending_runtime_recovery_action: &'a Arc<
            Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>,
        >,
        connection: &'a Arc<Mutex<RtcConnectionService>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
        local_decoder_reset_handle: &'a Arc<
            Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>,
        >,
        transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
        transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
    ) -> Self {
        Self {
            runtime_stats,
            runtime_config,
            pending_runtime_recovery_action,
            connection,
            media,
            local_decoder_reset_handle,
            transport_session,
            transport_fact_sink,
        }
    }

    pub(crate) fn pump_connection_and_media_ingress(&self) {
        crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection_and_media_ingress enter");
        self.record_transport_fact(TransportFact::Timer(TimerFact::MetricsSampleTick {
            observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
        }));
        let (ingress_packets, connection_facts) = self
            .connection
            .lock()
            .ok()
            .map(|mut connection| {
                crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection lock acquired");
                let _ = connection.pump(self.runtime_stats);
                (
                    connection.take_media_ingress_packets(),
                    connection.take_transport_facts(),
                )
            })
            .unwrap_or_default();
        for fact in connection_facts {
            self.record_transport_fact(fact);
        }
        crate::xbx_log_debug!(
            "[xbxengine][rtc-stack] pump_connection_and_media_ingress ingress_packets={}",
            ingress_packets.len()
        );
        if !ingress_packets.is_empty() {
            if let Ok(mut media) = self.media.lock() {
                for (packet, rtp_meta) in ingress_packets {
                    // 第一阶段只把连接层原始包送进媒体入口，不进入组帧/送解链。
                    media.observe_ingress_packet(packet, rtp_meta, self.runtime_stats);
                }
            }
        }
        self.drain_transport_fact_sink();
        crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection_and_media_ingress exit");
    }

    pub(crate) fn record_transport_fact(&self, fact: TransportFact) {
        let mut pending_commands = Vec::new();
        if let Ok(mut session) = self.transport_session.lock() {
            session.enqueue_fact(fact);
            let _ = session.drain_once(64);
            while let Some(command) = session.pop_next_command() {
                pending_commands.push(command);
            }
        }
        for command in pending_commands {
            self.apply_transport_session_command(command);
        }
    }

    pub(crate) fn reset_transport_session(&self) {
        if let Ok(mut session) = self.transport_session.lock() {
            *session = SessionActor::new(
                SystemSessionClock,
                RtcSessionPolicy::new(self.runtime_config.clone(), self.runtime_stats.clone()),
            );
        }
    }

    pub(crate) fn drain_transport_fact_sink(&self) {
        let facts = self
            .transport_fact_sink
            .lock()
            .ok()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default();
        for fact in facts {
            self.record_transport_fact(fact);
        }
    }

    pub(crate) fn record_transport_command_result(
        &self,
        command: TransportCommand,
        result: &Result<(), XbxEngineRuntimeError>,
    ) {
        let status = match result {
            Ok(()) => CommandResultStatus::Succeeded,
            Err(error) => CommandResultStatus::Failed {
                error: error.to_string(),
            },
        };
        self.record_transport_command_status(command, status);
    }

    pub(crate) fn record_transport_command_status(
        &self,
        command: TransportCommand,
        status: CommandResultStatus,
    ) {
        self.record_transport_command_status_with_semantic(command, status, None);
    }

    fn record_transport_command_status_with_semantic(
        &self,
        command: TransportCommand,
        status: CommandResultStatus,
        semantic_detail: Option<String>,
    ) {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        // CommandResult 若同步回灌到 session，会在同一调用栈里触发
        // apply_command -> command_result -> apply_command 的递归环。
        // 这里统一改为入队，交给下一轮 drain 处理，避免 tokio worker 栈溢出。
        if let Ok(mut pending) = self.transport_fact_sink.lock() {
            pending.push(TransportFact::CommandResult(CommandResultFact {
                command: command.clone(),
                status: status.clone(),
                observed_at_ms,
            }));
        }
        self.update_recovery_decision_command_result(
            &command,
            &status,
            observed_at_ms,
            semantic_detail.as_deref(),
        );
        self.record_transport_command_semantic_observation(
            &command,
            &status,
            semantic_detail.as_deref(),
            observed_at_ms,
        );
    }

    fn update_recovery_decision_semantic_fields(
        &self,
        decision_id: u64,
        fields: &RecoveryCommandDecisionSemantics,
        observed_at_ms: f64,
    ) {
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            let apply_update = |ledger: &mut crate::XbxEngineRecoveryDecisionLedgerObservation| {
                if let Some(value) = fields.frame_value {
                    ledger.frame_value = Some(value.to_string());
                }
                if let Some(value) = fields.gap_severity {
                    ledger.gap_severity = Some(value.to_string());
                }
                if let Some(value) = fields.recovery_episode_stage {
                    ledger.recovery_episode_stage = Some(value.to_string());
                }
                if let Some(value) = fields.recovery_episode_progress_at_ms {
                    ledger.recovery_episode_progress_at_ms = Some(value);
                }
                if let Some(value) = fields.coalescing_mode {
                    ledger.coalescing_mode = Some(value.to_string());
                }
                if let Some(value) = fields.unlock_reason {
                    ledger.unlock_reason = Some(value.to_string());
                }
                if let Some(value) = fields.preempt_reason {
                    ledger.preempt_reason = Some(value.to_string());
                }
                if let Some(value) = fields.recovery_primary_action {
                    ledger.recovery_primary_action = Some(value.to_string());
                }
                ledger.observed_at_ms = observed_at_ms;
            };
            if let Some(index) = stats
                .recent_recovery_decision_ledgers
                .iter()
                .rposition(|ledger| ledger.decision_id == decision_id)
            {
                apply_update(&mut stats.recent_recovery_decision_ledgers[index]);
            }
            if let Some(ledger) = stats.latest_recovery_decision_ledger.as_mut() {
                if ledger.decision_id != decision_id {
                    return;
                }
                apply_update(ledger);
            }
        });
    }

    pub(crate) fn apply_transport_session_command(&self, command: SessionCommand) {
        match &command {
            SessionCommand::LocalDecoderReset {
                observation_id,
                reason,
            } => {
                let now_ms = crate::transport::rtc::stats::now_ms_f64();
                let (family_decision, family_semantics, family_semantic_detail) = self
                    .resolve_recovery_command_family_decision(
                        RecoveryCommandKind::RequestDecoderReset,
                        Some(reason.as_str()),
                        now_ms,
                    );
                let family_upgrade_detail = match &family_decision {
                    RecoveryCommandFamilyDecision::Defer { reason } => {
                        if let Some(fields) = family_semantics.as_ref() {
                            self.update_recovery_decision_semantic_fields(
                                *observation_id,
                                fields,
                                now_ms,
                            );
                        }
                        self.record_local_decoder_reset_result(
                            *observation_id,
                            reason.clone(),
                            CommandResultStatus::Deferred {
                                reason: reason.clone(),
                            },
                            family_semantic_detail,
                        );
                        if reason
                            .contains(RECOVERY_COMMAND_REASON_SAME_FAMILY_DECODER_RESET_COALESCED)
                        {
                            if let Ok(mut pending) = self.transport_fact_sink.lock() {
                                pending.push(TransportFact::Recovery(
                                    RecoveryEscalationFact::DecoderResetFamilyCoalesceDeferred,
                                ));
                            }
                            self.drain_transport_fact_sink();
                        }
                        return;
                    }
                    RecoveryCommandFamilyDecision::Refresh { detail } => Some(detail.clone()),
                    RecoveryCommandFamilyDecision::Upgrade { detail } => Some(detail.clone()),
                    RecoveryCommandFamilyDecision::Proceed => None,
                };
                if let Some(fields) = family_semantics.as_ref() {
                    self.update_recovery_decision_semantic_fields(*observation_id, fields, now_ms);
                }
                let command_status =
                    match self.request_local_decoder_reset(format!("recoveryCommand:{reason}")) {
                        Ok(()) => CommandResultStatus::Succeeded,
                        Err(error) => CommandResultStatus::Deferred {
                            reason: error.to_string(),
                        },
                    };
                if matches!(command_status, CommandResultStatus::Succeeded) {
                    self.record_recovery_escalation_observation(
                        *observation_id,
                        reason.clone(),
                        RecoveryAction::RequestDecoderReset.label().to_string(),
                        RecoveryAction::RequestDecoderReset,
                    );
                }
                self.record_local_decoder_reset_result(
                    *observation_id,
                    reason.clone(),
                    command_status,
                    family_upgrade_detail,
                );
            }
            SessionCommand::Transport(command) => match command {
                TransportCommand::RequestReconnectCandidate {
                    observation_id,
                    reason,
                    reason_domain,
                } => {
                    let result = self
                        .pending_runtime_recovery_action
                        .lock()
                        .map_err(|_| {
                            XbxEngineRuntimeError::new(
                                "xbxEngineRtcPendingRecoveryActionLockFailed",
                            )
                        })
                        .map(|mut pending| {
                            let stage_outcome = stage_reconnect_candidate(
                                &mut pending,
                                *observation_id,
                                reason.clone(),
                                *reason_domain,
                            );
                            let pending_reason = pending.as_ref().map(|action| {
                                match action {
                            XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                                reason,
                                ..
                            } => reason.clone(),
                        }
                            });
                            (stage_outcome, pending_reason)
                        });
                    if result.as_ref().is_ok_and(|(stage_outcome, _)| {
                        !matches!(stage_outcome, StageReconnectCandidateOutcome::Unchanged)
                    }) {
                        self.record_recovery_escalation_observation(
                            *observation_id,
                            reason.clone(),
                            "requestReconnectCandidate".to_string(),
                            RecoveryAction::RequestReconnectCandidate,
                        );
                    }
                    RuntimeStatsSink::update_shared(self.runtime_stats, |stats| {
                        match result.as_ref() {
                            Ok((
                                StageReconnectCandidateOutcome::StagedNew
                                | StageReconnectCandidateOutcome::StagedUpdated,
                                pending_reason,
                            )) => {
                                stats.latest_observation_label =
                                    Some("rtcReconnectCandidateStaged".to_string());
                                stats.latest_observation_summary = Some(format!(
                                    "observationId={} reason={} pendingReason={}",
                                    observation_id,
                                    reason,
                                    pending_reason.as_deref().unwrap_or("none")
                                ));
                            }
                            Ok((StageReconnectCandidateOutcome::Unchanged, pending_reason)) => {
                                stats.latest_observation_label =
                                    Some("rtcReconnectCandidateRejected".to_string());
                                stats.latest_observation_summary = Some(format!(
                                    "observationId={} reason={} pendingReason={} staged=false",
                                    observation_id,
                                    reason,
                                    pending_reason.as_deref().unwrap_or("none")
                                ));
                            }
                            Err(error) => {
                                stats.latest_observation_label =
                                    Some("rtcReconnectCandidateStageFailed".to_string());
                                stats.latest_observation_summary =
                                    Some(format!("reason={} error={error}", reason));
                            }
                        }
                    });
                    let command_result = match result.as_ref() {
                        Ok((
                            StageReconnectCandidateOutcome::StagedNew
                            | StageReconnectCandidateOutcome::StagedUpdated,
                            _,
                        )) => CommandResultStatus::Succeeded,
                        Ok((StageReconnectCandidateOutcome::Unchanged, pending_reason)) => {
                            CommandResultStatus::Deferred {
                                reason: format!(
                                    "pendingReason={}",
                                    pending_reason.as_deref().unwrap_or("none")
                                ),
                            }
                        }
                        Err(error) => CommandResultStatus::Failed {
                            error: error.to_string(),
                        },
                    };
                    self.record_transport_command_status(command.clone(), command_result);
                }
                TransportCommand::RequestKeyframe {
                    observation_id,
                    reason,
                } => {
                    let requested_at_ms = crate::transport::rtc::stats::now_ms_f64();
                    let (family_decision, family_semantics, family_semantic_detail) = self
                        .resolve_recovery_command_family_decision(
                            RecoveryCommandKind::RequestKeyframe,
                            Some(reason.as_str()),
                            requested_at_ms,
                        );
                    match family_decision {
                        RecoveryCommandFamilyDecision::Defer { reason } => {
                            if let Some(fields) = family_semantics.as_ref() {
                                self.update_recovery_decision_semantic_fields(
                                    *observation_id,
                                    fields,
                                    requested_at_ms,
                                );
                            }
                            self.record_transport_command_status(
                                command.clone(),
                                CommandResultStatus::Deferred { reason },
                            );
                            return;
                        }
                        RecoveryCommandFamilyDecision::Upgrade { .. }
                        | RecoveryCommandFamilyDecision::Refresh { .. }
                        | RecoveryCommandFamilyDecision::Proceed => {}
                    }
                    RuntimeStatsSink::new(self.runtime_stats.clone())
                        .record_keyframe_request_episode_requested(
                            *observation_id,
                            Some(reason.clone()),
                            requested_at_ms,
                            None,
                        );
                    let result = self
                        .connection
                        .lock()
                        .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                        .and_then(|mut connection| {
                            connection.request_video_keyframe(self.runtime_stats)
                        });
                    let latest_observation_label =
                        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                            stats.latest_observation_label.clone()
                        })
                        .flatten();
                    let (command_status, semantic_detail) = self
                        .resolve_recovery_command_status_from_result(
                            RecoveryCommandKind::RequestKeyframe,
                            &result,
                            latest_observation_label.as_deref(),
                        );
                    if matches!(command_status, CommandResultStatus::Succeeded) {
                        if let Some(action) = self.resolve_recovery_keyframe_action_label() {
                            self.record_recovery_escalation_observation(
                                *observation_id,
                                reason.clone(),
                                action,
                                RecoveryAction::RequestKeyframe,
                            );
                        }
                    }
                    match &command_status {
                        CommandResultStatus::Deferred { reason } => {
                            RuntimeStatsSink::new(self.runtime_stats.clone())
                                .record_keyframe_request_episode_deferred(requested_at_ms, reason);
                        }
                        CommandResultStatus::Failed { error } => {
                            RuntimeStatsSink::new(self.runtime_stats.clone())
                                .record_keyframe_request_episode_failed(requested_at_ms, error);
                        }
                        CommandResultStatus::Succeeded => {}
                    }
                    if let Some(fields) = family_semantics.as_ref() {
                        self.update_recovery_decision_semantic_fields(
                            *observation_id,
                            fields,
                            requested_at_ms,
                        );
                    }
                    self.record_transport_command_status_with_semantic(
                        command.clone(),
                        command_status,
                        match (family_semantic_detail.as_deref(), semantic_detail) {
                            (Some(family), Some(mut tail)) => {
                                tail.push_str(" | family=");
                                tail.push_str(family);
                                Some(tail)
                            }
                            (Some(family), None) => Some(format!("family={family}")),
                            (None, some) => some,
                        },
                    );
                }
                TransportCommand::RequestDecoderReset {
                    observation_id,
                    reason,
                } => {
                    self.apply_transport_session_command(SessionCommand::LocalDecoderReset {
                        observation_id: *observation_id,
                        reason: reason.clone(),
                    });
                }
                TransportCommand::SetTargetRembKbps {
                    target_kbps,
                    reason,
                    observation_id,
                } => {
                    let result = self
                        .connection
                        .lock()
                        .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                        .and_then(|mut connection| {
                            connection.request_target_remb_kbps(*target_kbps, self.runtime_stats)
                        });
                    let bwe_mode = self
                        .runtime_config
                        .lock()
                        .ok()
                        .map(|config| config.webrtc.bwe_mode.clone())
                        .unwrap_or_else(|| "fixed-remb".to_string());
                    let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
                    let target_kbps = *target_kbps;
                    let observation_id = *observation_id;
                    let decision_reason = reason.clone();
                    RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                        let twcc = stats.latest_video_twcc_observation.clone();
                        let observed_remb_kbps = stats.video_remb_bps.map(|bps| bps / 1_000);
                        let actual_video_bitrate_kbps = twcc
                            .as_ref()
                            .and_then(|value| value.receive_bitrate_kbps)
                            .or(stats.inbound_video_bitrate_kbps)
                            .unwrap_or(0.0)
                            .max(0.0);
                        stats.video_remb_bps = Some(target_kbps.saturating_mul(1_000));
                        stats.latest_video_bwe_observation = Some(XbxEngineVideoBweObservation {
                            observation_id,
                            mode: bwe_mode.clone(),
                            decision_reason: decision_reason.clone(),
                            target_remb_kbps: target_kbps,
                            observed_remb_kbps,
                            actual_video_bitrate_kbps,
                            loss_ratio: stats.inbound_video_loss_ratio_1s.clamp(0.0, 1.0),
                            rtt_ms: stats.video_rtt_ms,
                            transport_path: stats.transport_path.clone(),
                            twcc_feedback_interval_ms: twcc
                                .as_ref()
                                .and_then(|value| value.feedback_interval_ms),
                            twcc_observed_packet_count: twcc
                                .as_ref()
                                .map(|value| value.observed_packet_count),
                            twcc_covered_sequence_span: twcc
                                .as_ref()
                                .map(|value| value.covered_sequence_span),
                            twcc_receive_bitrate_kbps: twcc
                                .as_ref()
                                .and_then(|value| value.receive_bitrate_kbps),
                            twcc_delivery_ratio: twcc.as_ref().map(|value| value.delivery_ratio),
                            twcc_loss_ratio: twcc.as_ref().map(|value| value.packet_loss_ratio),
                            observed_at_ms,
                        });
                        stats.latest_observation_label =
                            Some("rtcSessionCommandUpdateTargetRemb".to_string());
                        stats.latest_observation_summary = Some(format!(
                            "rtc session command updated target remb={}kbps reason={}",
                            target_kbps, decision_reason
                        ));
                    });
                    self.record_transport_command_result(command.clone(), &result);
                }
            },
        }
    }

    fn record_local_decoder_reset_result(
        &self,
        observation_id: u64,
        _reason: String,
        status: CommandResultStatus,
        semantic_detail: Option<String>,
    ) {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        self.update_recovery_decision_result(
            observation_id,
            "requestDecoderReset",
            &status,
            observed_at_ms,
            semantic_detail.as_deref(),
        );
        self.record_command_semantic_observation(
            "requestDecoderReset",
            &status,
            semantic_detail.as_deref(),
            observed_at_ms,
        );
    }

    fn queue_local_decoder_reset(&self, reason: String, observed_at_ms: f64) -> bool {
        let local_decoder_handle = self
            .local_decoder_reset_handle
            .lock()
            .ok()
            .and_then(|handle| handle.clone());
        RuntimeStatsSink::update_shared(self.runtime_stats, |stats| {
            stats.latest_observation_label = Some(
                if local_decoder_handle.is_some() {
                    "videoDecoderLocalResetQueued"
                } else {
                    "videoDecoderLocalResetSkipped"
                }
                .to_string(),
            );
            stats.latest_observation_summary = Some(format!(
                "reason={reason} source=recoveryCommand observedAtMs={observed_at_ms:.3}"
            ));
        });
        if let Some(handle) = local_decoder_handle {
            handle.request_local_decoder_reset(reason, observed_at_ms);
            return true;
        }
        false
    }

    pub(crate) fn request_local_decoder_reset(
        &self,
        reason: String,
    ) -> Result<(), XbxEngineRuntimeError> {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        if self.queue_local_decoder_reset(reason, observed_at_ms) {
            return Ok(());
        }
        Err(XbxEngineRuntimeError::new(
            "xbxEngineRtcLocalDecoderResetHandleUnavailable",
        ))
    }

    fn record_recovery_escalation_observation(
        &self,
        observation_id: u64,
        reason: String,
        action: String,
        recovery_action: RecoveryAction,
    ) {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        let advances_recovery_epoch = self
            .should_advance_transport_recovery_epoch_on_success(recovery_action, reason.as_str());
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_recovery_escalation_success(
            observation_id,
            reason,
            action.as_str(),
            observed_at_ms,
            advances_recovery_epoch,
        );
    }

    fn should_advance_transport_recovery_epoch_on_success(
        &self,
        recovery_action: RecoveryAction,
        reason_label: &str,
    ) -> bool {
        VideoEscalationController::action_success_advances_transport_recovery_epoch(
            recovery_action,
            // 边界入站：栈层仅持有 wire label，映射到枚举以复用 coordinator 合同。
            VideoEscalationReason::from_recovery_reason_label(reason_label),
        )
    }

    fn resolve_recovery_keyframe_action_label(&self) -> Option<String> {
        let latest_observation_label = RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
            stats.latest_observation_label.clone()
        })
        .flatten();
        match latest_observation_label.as_deref() {
            Some("rtcVideoPliRequested") => Some("requestKeyframe(pli)".to_string()),
            Some("rtcVideoFirRequested") => Some("requestKeyframe(fir)".to_string()),
            Some("rtcVideoRecoverySuppressed") => None,
            Some("rtcVideoPliFallbackControl")
            | Some("rtcVideoFirFallbackControl")
            | Some("rtcControlKeyframeRequested") => Some("requestKeyframe".to_string()),
            Some(_) | None => Some("requestKeyframe".to_string()),
        }
    }

    fn update_recovery_decision_command_result(
        &self,
        command: &TransportCommand,
        status: &CommandResultStatus,
        observed_at_ms: f64,
        semantic_detail: Option<&str>,
    ) {
        let decision_id = match command {
            TransportCommand::RequestKeyframe { observation_id, .. }
            | TransportCommand::RequestDecoderReset { observation_id, .. }
            | TransportCommand::RequestReconnectCandidate { observation_id, .. }
            | TransportCommand::SetTargetRembKbps { observation_id, .. } => *observation_id,
        };
        let command_name = match command {
            TransportCommand::RequestKeyframe { .. } => "requestKeyframe",
            TransportCommand::RequestDecoderReset { .. } => "requestDecoderReset",
            TransportCommand::RequestReconnectCandidate { .. } => "requestReconnectCandidate",
            TransportCommand::SetTargetRembKbps { .. } => "setTargetRembKbps",
        };
        self.update_recovery_decision_result(
            decision_id,
            command_name,
            status,
            observed_at_ms,
            semantic_detail,
        );
    }

    fn update_recovery_decision_result(
        &self,
        decision_id: u64,
        command_name: &str,
        status: &CommandResultStatus,
        observed_at_ms: f64,
        semantic_detail: Option<&str>,
    ) {
        let (result_label, detail) = match status {
            CommandResultStatus::Succeeded => ("succeeded".to_string(), None),
            CommandResultStatus::Deferred { reason } => {
                ("deferred".to_string(), Some(reason.clone()))
            }
            CommandResultStatus::Failed { error } => ("failed".to_string(), Some(error.clone())),
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            let apply_update = |ledger: &mut crate::XbxEngineRecoveryDecisionLedgerObservation| {
                ledger.command_result = Some(result_label.clone());
                ledger.command_detail = Some(self.build_command_detail(
                    command_name,
                    detail.as_deref(),
                    semantic_detail,
                ));
                ledger.observed_at_ms = observed_at_ms;
            };
            if let Some(index) = stats
                .recent_recovery_decision_ledgers
                .iter()
                .rposition(|ledger| ledger.decision_id == decision_id)
            {
                apply_update(&mut stats.recent_recovery_decision_ledgers[index]);
            }
            if let Some(ledger) = stats.latest_recovery_decision_ledger.as_mut() {
                if ledger.decision_id != decision_id {
                    return;
                }
                apply_update(ledger);
            }
        });
    }

    fn build_command_detail(
        &self,
        command_name: &str,
        status_detail: Option<&str>,
        semantic_detail: Option<&str>,
    ) -> String {
        let mut detail = format!("command={command_name}");
        if let Some(raw) = status_detail {
            detail.push_str(" detail=");
            detail.push_str(raw);
        }
        if let Some(semantic) = semantic_detail {
            detail.push_str(" semantic=");
            detail.push_str(semantic);
        }
        detail
    }

    fn record_transport_command_semantic_observation(
        &self,
        command: &TransportCommand,
        status: &CommandResultStatus,
        semantic_detail: Option<&str>,
        observed_at_ms: f64,
    ) {
        let command_name = match command {
            TransportCommand::RequestKeyframe { .. } => "requestKeyframe",
            TransportCommand::RequestDecoderReset { .. } => "requestDecoderReset",
            TransportCommand::RequestReconnectCandidate { .. } => "requestReconnectCandidate",
            TransportCommand::SetTargetRembKbps { .. } => "setTargetRembKbps",
        };
        self.record_command_semantic_observation(
            command_name,
            status,
            semantic_detail,
            observed_at_ms,
        );
    }

    fn record_command_semantic_observation(
        &self,
        command_name: &str,
        status: &CommandResultStatus,
        semantic_detail: Option<&str>,
        observed_at_ms: f64,
    ) {
        let status_name = match status {
            CommandResultStatus::Succeeded => "succeeded",
            CommandResultStatus::Deferred { .. } => "deferred",
            CommandResultStatus::Failed { .. } => "failed",
        };
        let status_detail = match status {
            CommandResultStatus::Deferred { reason } => Some(reason.as_str()),
            CommandResultStatus::Failed { error } => Some(error.as_str()),
            CommandResultStatus::Succeeded => None,
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_transport_command_semantic(
            command_name,
            status_name,
            status_detail,
            semantic_detail,
            observed_at_ms,
        );
    }

    fn resolve_recovery_command_status_from_result(
        &self,
        command_kind: RecoveryCommandKind,
        result: &Result<(), XbxEngineRuntimeError>,
        latest_observation_label: Option<&str>,
    ) -> (CommandResultStatus, Option<String>) {
        match result {
            Ok(()) => {
                if matches!(command_kind, RecoveryCommandKind::RequestKeyframe)
                    && latest_observation_label == Some("rtcVideoRecoverySuppressed")
                {
                    return (
                        CommandResultStatus::Deferred {
                            reason: RECOVERY_COMMAND_REASON_SAME_FAMILY_TRANSPORT_STAGE_COALESCED
                                .to_string(),
                        },
                        None,
                    );
                }
                (CommandResultStatus::Succeeded, None)
            }
            Err(error) => {
                let error_text = error.to_string();
                if self.is_control_channel_not_ready_error(command_kind, &error_text) {
                    return (
                        CommandResultStatus::Deferred {
                            reason: RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_CONTROL_PENDING
                                .to_string(),
                        },
                        None,
                    );
                }
                (CommandResultStatus::Failed { error: error_text }, None)
            }
        }
    }

    fn is_control_channel_not_ready_error(
        &self,
        command_kind: RecoveryCommandKind,
        error: &str,
    ) -> bool {
        match command_kind {
            RecoveryCommandKind::RequestKeyframe => {
                error.contains("xbxEngineRtcControlChannelNotReadyForKeyframe")
            }
            RecoveryCommandKind::RequestDecoderReset => {
                error.contains("xbxEngineRtcControlChannelNotReadyForDecoderReset")
            }
        }
    }

    fn resolve_recovery_command_family_decision(
        &self,
        command_kind: RecoveryCommandKind,
        reason_label: Option<&str>,
        now_ms: f64,
    ) -> (
        RecoveryCommandFamilyDecision,
        Option<RecoveryCommandDecisionSemantics>,
        Option<String>,
    ) {
        fn keyframe_reason_family(reason: &str) -> &'static str {
            match reason {
                // transport await / wait-keyframe 族：共享同一 in-flight 保护与解锁语义
                "waitKeyframe"
                | "ingressWaitKeyframe"
                | "ingressFrameAbandoned"
                | "frameAbandoned"
                | "transportAwaitRecoveryKeyframe"
                | "bootstrapMissingSps"
                | "bootstrapMissingPps"
                | "inspectionRejectInvalidSliceHeader" => "keyframe-recovery",
                // display supply / adapter 等本地修复族
                "displaySupplyDegraded" | "displaySupplyCritical" | "adapterThinStream" => {
                    "display-supply"
                }
                _ => "generic",
            }
        }

        fn episode_matches_reason_family(
            episode: &crate::XbxEngineKeyframeRequestEpisodeObservation,
            desired_family: Option<&str>,
        ) -> bool {
            desired_family.is_none_or(|desired| {
                episode
                    .request_reason
                    .as_deref()
                    .is_some_and(|request_reason| keyframe_reason_family(request_reason) == desired)
            })
        }

        let desired_family = reason_label.map(keyframe_reason_family);
        let (
            keyframe_in_flight,
            keyframe_unlock_reason,
            decoder_reset_in_flight,
            decoder_reset_unlock_reason,
            derived_episode_stage,
            derived_gap_severity,
            derived_frame_value,
            keyframe_refresh_candidate,
        ) = RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let mut unlock_reason = None;
            let matching_episode = stats
                .latest_keyframe_request_episode
                .as_ref()
                .filter(|episode| episode_matches_reason_family(episode, desired_family));
            let keyframe_in_flight = matching_episode.is_some_and(|episode| {
                // 仅“真正仍在推进恢复闭环”的 episode 才能占坑。
                let holdable_verdict = !matches!(
                    episode.response_verdict.as_deref(),
                    Some("transportDeferred" | "transportFailed" | "missed")
                );
                let in_flight_status = matches!(
                    episode.status.as_str(),
                    "requested" | "sent" | "response-observed" | "decoded"
                ) && episode.sent_at_ms.is_some();
                let anchor_at_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
                let within_window =
                    (now_ms - anchor_at_ms).max(0.0) <= RECOVERY_COMMAND_FAMILY_IN_FLIGHT_WINDOW_MS;
                if !(holdable_verdict && in_flight_status && within_window) {
                    return false;
                }

                // sent 后长期没有任何推进边沿（首包/响应/解码/clean anchor），属于 stalled，占坑无意义。
                if episode.sent_at_ms.is_some_and(|sent_at_ms| {
                    (now_ms - sent_at_ms).max(0.0) > KEYFRAME_NO_PROGRESS_STALL_MS
                        && episode.first_video_packet_at_ms.is_none()
                        && episode.first_keyframe_packet_at_ms.is_none()
                        && episode.first_keyframe_decoded_at_ms.is_none()
                }) {
                    unlock_reason = Some("episodeStalledNoProgress");
                    return false;
                }

                // 看到明确的 NonIdrVcl 等无效响应时，不应继续占坑压制新动作。
                if stats
                    .latest_h264_inspection_observation
                    .as_ref()
                    .is_some_and(|inspection| {
                        inspection.observed_at_ms >= anchor_at_ms
                            && !inspection.bootstrap_ready
                            && matches!(
                                inspection.bootstrap_reject_reason.as_deref(),
                                Some(
                                    "NonIdrVcl"
                                        | "bootstrapMissingSps"
                                        | "bootstrapMissingPps"
                                        | "inspectionRejectInvalidSliceHeader"
                                )
                            )
                    })
                {
                    unlock_reason = Some("bootstrapRejected:invalidBootstrap");
                    return false;
                }

                // 已解码但未形成 clean anchor 证据：只允许短暂占坑，避免“假在飞”。
                if episode.status == "decoded"
                    && !matches!(
                        stats.video_anchor_clean_epoch,
                        Some(epoch) if epoch == stats.transport_recovery_epoch
                    )
                {
                    let decoded_at_ms =
                        episode.first_keyframe_decoded_at_ms.unwrap_or(anchor_at_ms);
                    let elapsed_ms = (now_ms - decoded_at_ms).max(0.0);
                    if elapsed_ms > KEYFRAME_DECODED_PENDING_COMMIT_HOLD_MS {
                        unlock_reason = Some("decodedPendingCommitExpired");
                    }
                    return elapsed_ms <= KEYFRAME_DECODED_PENDING_COMMIT_HOLD_MS;
                }

                true
            });
            let keyframe_refresh_candidate = matching_episode.is_some()
                && !keyframe_in_flight
                && matching_episode.is_some_and(|episode| {
                    let has_clean_anchor_evidence = matches!(
                        stats.video_anchor_clean_epoch,
                        Some(epoch) if epoch == stats.transport_recovery_epoch
                    );
                    if is_terminal_transport_await_deferred_episode(
                        episode,
                        stats.latest_h264_inspection_observation.as_ref(),
                        has_clean_anchor_evidence,
                        now_ms,
                        220.0,
                    ) {
                        unlock_reason = Some("terminalDeferredInvalidBootstrap");
                        return false;
                    }
                    let anchor_at_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
                    (now_ms - anchor_at_ms).max(0.0) <= RECOVERY_COMMAND_FAMILY_IN_FLIGHT_WINDOW_MS
                });
            let mut decoder_unlock_reason = None;
            let decoder_reset_in_flight = latest_decoder_reset_family_attempt_at_ms(stats)
                .is_some_and(|attempt_at_ms| {
                    if (now_ms - attempt_at_ms).max(0.0) > DECODER_RESET_PROGRESS_HOLD_MS {
                        decoder_unlock_reason = Some("decoderResetProgressExpired");
                        return false;
                    }
                    if has_post_decoder_reset_progress(stats, attempt_at_ms) {
                        decoder_unlock_reason = Some("decoderResetProgressObserved");
                        return false;
                    }
                    if has_transport_await_invalid_keyframe_response_after_decoder_reset(
                        stats,
                        attempt_at_ms,
                        now_ms,
                    ) {
                        decoder_unlock_reason = Some("decoderResetInvalidRecoveryResponse");
                        return false;
                    }
                    true
                });
            let episode_stage = matching_episode.and_then(|episode| {
                // 优先用 clean anchor 证据判断完成态。
                if matches!(
                    stats.video_anchor_clean_epoch,
                    Some(epoch) if epoch == stats.transport_recovery_epoch
                ) {
                    return Some(RecoveryEpisodeStage::CleanAnchorCommitted.as_str());
                }
                if unlock_reason == Some("episodeStalledNoProgress") {
                    return Some(RecoveryEpisodeStage::Stalled.as_str());
                }
                match episode.status.as_str() {
                    "requested" => Some(RecoveryEpisodeStage::Requested.as_str()),
                    "sent" => Some(RecoveryEpisodeStage::Sent.as_str()),
                    "response-observed" | "packet-seen" => {
                        Some(RecoveryEpisodeStage::ResponseObserved.as_str())
                    }
                    "decoded" => Some(RecoveryEpisodeStage::Decoded.as_str()),
                    "deferred" => Some(RecoveryEpisodeStage::Deferred.as_str()),
                    "expired-unsent" | "missed" => Some(RecoveryEpisodeStage::Expired.as_str()),
                    _ => None,
                }
            });

            let uses_transport_await_semantics = desired_family == Some("keyframe-recovery")
                || matching_episode.is_some_and(|episode| {
                    episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                });
            let gap_model = uses_transport_await_semantics
                .then(|| {
                    stats
                        .latest_video_timeline_observation
                        .as_ref()
                        .map(|timeline| {
                            derive_gap_severity_with_episode_stall(
                                timeline,
                                unlock_reason == Some("episodeStalledNoProgress"),
                            )
                        })
                })
                .flatten();
            let gap_severity = gap_model.map(|gs| gs.as_str());
            let frame_value =
                gap_model.and_then(|gs| frame_value_from_gap_severity(gs).map(|fv| fv.as_str()));

            (
                keyframe_in_flight,
                unlock_reason,
                decoder_reset_in_flight,
                decoder_unlock_reason,
                episode_stage,
                gap_severity,
                frame_value,
                keyframe_refresh_candidate,
            )
        })
        .unwrap_or((false, None, false, None, None, None, None, false));

        let mut semantics = RecoveryCommandDecisionSemantics::default();
        semantics.recovery_primary_action = Some(match command_kind {
            RecoveryCommandKind::RequestKeyframe => "requestKeyframe",
            RecoveryCommandKind::RequestDecoderReset => "requestDecoderReset",
        });
        semantics.unlock_reason = keyframe_unlock_reason.or(decoder_reset_unlock_reason);
        semantics.recovery_episode_stage = derived_episode_stage;
        semantics.gap_severity = derived_gap_severity;
        semantics.frame_value = derived_frame_value;
        semantics.recovery_episode_progress_at_ms =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .and_then(|episode| {
                        if !episode_matches_reason_family(episode, desired_family) {
                            return None;
                        }
                        let anchor_at_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
                        Some((now_ms - anchor_at_ms).max(0.0))
                    })
            })
            .flatten();

        let (decision, semantic_detail) = match command_kind {
            RecoveryCommandKind::RequestKeyframe => {
                if decoder_reset_in_flight {
                    semantics.coalescing_mode = Some("Merge");
                    (
                        RecoveryCommandFamilyDecision::Defer {
                            reason: RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_DECODER_RESET
                                .to_string(),
                        },
                        Some("family=decoderResetInFlight".to_string()),
                    )
                } else if keyframe_refresh_candidate {
                    semantics.coalescing_mode = Some("Refresh");
                    (
                        RecoveryCommandFamilyDecision::Refresh {
                            detail: RECOVERY_COMMAND_REASON_SAME_FAMILY_KEYFRAME_REFRESHED
                                .to_string(),
                        },
                        Some("sameFamilyRefreshed:keyframeEpisode".to_string()),
                    )
                } else if keyframe_in_flight {
                    semantics.coalescing_mode = Some("Merge");
                    (
                        RecoveryCommandFamilyDecision::Defer {
                            reason: RECOVERY_COMMAND_REASON_SAME_FAMILY_KEYFRAME_COALESCED
                                .to_string(),
                        },
                        Some("family=keyframeInFlight".to_string()),
                    )
                } else {
                    (RecoveryCommandFamilyDecision::Proceed, None)
                }
            }
            RecoveryCommandKind::RequestDecoderReset => {
                if decoder_reset_in_flight {
                    semantics.coalescing_mode = Some("Merge");
                    (
                        RecoveryCommandFamilyDecision::Defer {
                            reason: RECOVERY_COMMAND_REASON_SAME_FAMILY_DECODER_RESET_COALESCED
                                .to_string(),
                        },
                        Some("family=decoderResetInFlight".to_string()),
                    )
                } else if keyframe_in_flight {
                    semantics.coalescing_mode = Some("Preempt");
                    semantics.preempt_reason =
                        Some(RECOVERY_COMMAND_SEMANTIC_FAMILY_UPGRADE_KEYFRAME_TO_DECODER_RESET);
                    (
                        RecoveryCommandFamilyDecision::Upgrade {
                            detail:
                                RECOVERY_COMMAND_SEMANTIC_FAMILY_UPGRADE_KEYFRAME_TO_DECODER_RESET
                                    .to_string(),
                        },
                        Some("familyUpgrade=keyframeInFlight->decoderReset".to_string()),
                    )
                } else {
                    (RecoveryCommandFamilyDecision::Proceed, None)
                }
            }
        };

        let semantics = (semantics.coalescing_mode.is_some()
            || semantics.unlock_reason.is_some()
            || semantics.preempt_reason.is_some()
            || semantics.recovery_primary_action.is_some())
        .then_some(semantics);

        (decision, semantics, semantic_detail)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use crate::api::runtime::XbxEngineRuntimeConfig;
    use crate::media::video::decode::actor::{DecodeActorHandle, DecodeMsg};
    use crate::transport::rtc::connection::RtcConnectionService;
    use crate::transport::rtc::facts::{CommandResultStatus, SessionCommand, TransportCommand};
    use crate::transport::rtc::recovery::escalation::RecoveryAction;
    use crate::transport::rtc::session::actor::SessionActor;
    use crate::transport::rtc::session::clock::SystemSessionClock;
    use crate::transport::rtc::session::policy::RtcSessionPolicy;
    use crate::transport::rtc::stream::RtcMediaService;
    use crate::{
        XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction,
        XbxEngineVideoEscalationObservation,
    };

    use super::RtcTransportSessionBridge;

    fn build_bridge(
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    ) -> RtcTransportSessionBridge<'static> {
        build_bridge_with_local_decoder_reset_handle(
            runtime_stats,
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(None)),
        )
    }

    fn build_bridge_with_local_decoder_reset_handle(
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
        local_decoder_reset_handle: Arc<Mutex<Option<Arc<DecodeActorHandle>>>>,
    ) -> RtcTransportSessionBridge<'static> {
        let runtime_stats = Box::leak(Box::new(runtime_stats));
        let runtime_config = Box::leak(Box::new(Arc::new(Mutex::new(
            XbxEngineRuntimeConfig::default(),
        ))));
        let pending_runtime_recovery_action = Box::leak(Box::new(pending_runtime_recovery_action));
        let connection = Box::leak(Box::new(Arc::new(Mutex::new(
            RtcConnectionService::default(),
        ))));
        let media = Box::leak(Box::new(Arc::new(Mutex::new(RtcMediaService::default()))));
        let local_decoder_reset_handle = Box::leak(Box::new(local_decoder_reset_handle));
        let transport_session = Box::leak(Box::new(Arc::new(Mutex::new(SessionActor::new(
            SystemSessionClock,
            RtcSessionPolicy::new(runtime_config.clone(), runtime_stats.clone()),
        )))));
        let transport_fact_sink = Box::leak(Box::new(Arc::new(Mutex::new(Vec::new()))));

        RtcTransportSessionBridge::new(
            runtime_stats,
            runtime_config,
            pending_runtime_recovery_action,
            connection,
            media,
            local_decoder_reset_handle,
            transport_session,
            transport_fact_sink,
        )
    }

    #[test]
    fn queue_local_decoder_reset_marks_skipped_when_handle_missing() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        assert!(!bridge.queue_local_decoder_reset("recoveryCommand:test".to_string(), 321.0,));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("videoDecoderLocalResetSkipped")
        );
        assert!(snapshot
            .latest_observation_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("source=recoveryCommand")));
    }

    #[test]
    fn queue_local_decoder_reset_enqueues_message_when_handle_available() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
        let bridge = build_bridge_with_local_decoder_reset_handle(
            runtime_stats.clone(),
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(Some(handle))),
        );

        assert!(bridge.queue_local_decoder_reset("recoveryCommand:test".to_string(), 654.0,));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("videoDecoderLocalResetQueued")
        );
        drop(snapshot);
        let msg = rx.recv().expect("local decoder reset message");
        match msg {
            DecodeMsg::LocalDecoderReset {
                reason,
                observed_at_ms,
            } => {
                assert_eq!(reason, "recoveryCommand:test");
                assert_eq!(observed_at_ms, 654.0);
            }
            _ => panic!("unexpected decode message"),
        }
    }

    #[test]
    fn request_decoder_reset_command_enqueues_local_reset_without_connection_support() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
        let bridge = build_bridge_with_local_decoder_reset_handle(
            runtime_stats.clone(),
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(Some(handle))),
        );

        bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
            observation_id: 42,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
        });

        let msg = rx.recv().expect("local decoder reset message");
        match msg {
            DecodeMsg::LocalDecoderReset { reason, .. } => {
                assert_eq!(reason, "recoveryCommand:transportAwaitRecoveryKeyframe");
            }
            _ => panic!("unexpected decode message"),
        }
        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("videoDecoderLocalResetQueued")
        );
    }

    #[test]
    fn invalid_transport_await_response_releases_decoder_reset_family_gate() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 5;
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 70.0);
        stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
            observation_id: 41,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            action: "requestDecoderReset".to_string(),
            recovery_stage: "rebuilding-supply".to_string(),
            recovery_chain_value: "anchor".to_string(),
            recovery_failure_cost: "medium".to_string(),
            recovery_window_source: "hard-fallback-window".to_string(),
            observed_at_ms: now_ms - 60.0,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 18,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 260.0,
                sent_at_ms: Some(now_ms - 220.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(now_ms - 55.0),
                first_video_packet_rtp_timestamp: Some(7_001),
                first_video_packet_is_keyframe: Some(false),
                first_keyframe_packet_at_ms: Some(now_ms - 55.0),
                first_keyframe_decoded_at_ms: Some(now_ms - 50.0),
                response_rtp_timestamp: Some(7_001),
                response_frame_seq: Some(88),
                response_verdict: Some("on-time".to_string()),
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 77,
                frame_rtp_timestamp: Some(7_001),
                nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: true,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: Some(1920),
                sample_height: Some(1080),
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: true,
                observed_at_ms: now_ms - 45.0,
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 42,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
                    .to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestDecoderReset".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
        let bridge = build_bridge_with_local_decoder_reset_handle(
            runtime_stats.clone(),
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(Some(handle))),
        );

        bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
            observation_id: 42,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
        });

        let msg = rx.recv().expect("decoder reset should proceed");
        match msg {
            DecodeMsg::LocalDecoderReset { reason, .. } => {
                assert_eq!(reason, "recoveryCommand:transportAwaitRecoveryKeyframe");
            }
            _ => panic!("unexpected decode message"),
        }

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("bootstrapRejected:invalidBootstrap")
        );
        assert_eq!(ledger.command_result.as_deref(), Some("succeeded"));
        assert!(ledger.command_detail.as_deref().is_some_and(|detail| {
            !detail.contains("sameFamilyCoalesced:decoderResetInFlight")
        }));
    }

    /// decoder reset 后新鲜 NonIdrVcl：episode 尚未进入 packet-seen/decoded 也应解除 decoderResetInFlight，
    /// 避免同族合并无限压制后续 reset。
    #[test]
    fn invalid_transport_await_non_idr_vcl_unlocks_decoder_reset_gate_without_packet_seen_episode()
    {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 5;
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 70.0);
        stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
            observation_id: 41,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            action: "requestDecoderReset".to_string(),
            recovery_stage: "rebuilding-supply".to_string(),
            recovery_chain_value: "anchor".to_string(),
            recovery_failure_cost: "medium".to_string(),
            recovery_window_source: "hard-fallback-window".to_string(),
            observed_at_ms: now_ms - 60.0,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 19,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 260.0,
                sent_at_ms: Some(now_ms - 220.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: None,
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 78,
                frame_rtp_timestamp: Some(7_002),
                nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: true,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: Some(1920),
                sample_height: Some(1080),
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: true,
                observed_at_ms: now_ms - 45.0,
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 43,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
                    .to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestDecoderReset".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
        let bridge = build_bridge_with_local_decoder_reset_handle(
            runtime_stats.clone(),
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(Some(handle))),
        );

        bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
            observation_id: 43,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
        });

        let msg = rx.recv().expect("decoder reset should proceed");
        match msg {
            DecodeMsg::LocalDecoderReset { reason, .. } => {
                assert_eq!(reason, "recoveryCommand:transportAwaitRecoveryKeyframe");
            }
            _ => panic!("unexpected decode message"),
        }

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert!(
            ledger.unlock_reason.as_deref() == Some("decoderResetInvalidRecoveryResponse")
                || ledger.unlock_reason.as_deref() == Some("bootstrapRejected:invalidBootstrap"),
            "unexpected unlock_reason: {:?}",
            ledger.unlock_reason
        );
        assert_eq!(ledger.command_result.as_deref(), Some("succeeded"));
        assert!(ledger.command_detail.as_deref().is_some_and(|detail| {
            !detail.contains("sameFamilyCoalesced:decoderResetInFlight")
        }));
    }

    #[test]
    fn reconnect_candidate_records_escalation_observation_when_staged() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 42,
                reason: "recovering-stream".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let escalation = snapshot
            .latest_video_escalation_observation
            .as_ref()
            .expect("escalation should be recorded");
        assert_eq!(escalation.observation_id, 42);
        assert_eq!(escalation.reason, "recovering-stream");
        assert_eq!(escalation.action, "requestReconnectCandidate");
        assert_eq!(
            snapshot.transport_recovery_epoch_at_last_escalation,
            snapshot.transport_recovery_epoch
        );
    }

    #[test]
    fn reconnect_candidate_overwrites_pending_and_records_new_escalation() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(Some(
            XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                observation_id: 1,
                reason: "existing".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
        )));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 43,
                reason: "new-reason".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let escalation = snapshot
            .latest_video_escalation_observation
            .as_ref()
            .expect("escalation should be recorded");
        assert_eq!(escalation.observation_id, 43);
        assert_eq!(escalation.reason, "new-reason");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("rtcReconnectCandidateStaged")
        );
    }

    #[test]
    fn reconnect_candidate_stage_preserves_reason_domain_in_pending_action() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action.clone());

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 44,
                reason: "displaySupplyCritical".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
            },
        ));

        let pending = pending_runtime_recovery_action
            .lock()
            .expect("pending reconnect action lock");
        assert!(matches!(
            pending.as_ref(),
            Some(XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                observation_id: 44,
                reason,
                reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
            }) if reason == "displaySupplyCritical"
        ));
    }

    #[test]
    fn reconnect_advances_recovery_epoch_by_contract() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 2;
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 77,
                reason: "reconnect-needed".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let escalation = snapshot
            .latest_video_escalation_observation
            .as_ref()
            .expect("escalation should be recorded");
        assert_eq!(escalation.action, "requestReconnectCandidate");
        assert_eq!(snapshot.transport_recovery_epoch, 3);
        assert_eq!(snapshot.transport_recovery_epoch_at_last_escalation, 3);
    }

    #[test]
    fn transport_session_maps_local_decoder_reset_reason_to_non_advancing_epoch_policy() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action);

        assert!(!bridge.should_advance_transport_recovery_epoch_on_success(
            RecoveryAction::RequestDecoderReset,
            "displaySupplyDegraded",
        ));
        assert!(!bridge.should_advance_transport_recovery_epoch_on_success(
            RecoveryAction::RequestDecoderReset,
            "transportAwaitRecoveryKeyframe",
        ));
    }

    #[test]
    fn decoder_reset_is_deferred_when_control_reset_observation_is_recent() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 7;
        stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
            observation_id: 101,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            action: "requestDecoderReset".to_string(),
            recovery_stage: "rebuilding-supply".to_string(),
            recovery_chain_value: "anchor".to_string(),
            recovery_failure_cost: "high".to_string(),
            recovery_window_source: "hard-fallback-window".to_string(),
            observed_at_ms: now_ms,
        });
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
            observation_id: 202,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(snapshot.transport_recovery_epoch, 7);
        assert_eq!(
            snapshot
                .latest_video_escalation_observation
                .as_ref()
                .map(|obs| obs.observation_id),
            Some(101)
        );
    }

    #[test]
    fn unsent_requested_keyframe_does_not_hold_family_gate() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: None,
                status: "requested".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: None,
                deadline_at_ms: None,
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 22,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let episode = snapshot
            .latest_keyframe_request_episode
            .as_ref()
            .expect("new episode should be recorded");
        assert_eq!(episode.episode_id, 22);
        assert_eq!(episode.status, "deferred");
        assert_eq!(
            episode.response_verdict.as_deref(),
            Some("transportDeferred")
        );
    }

    #[test]
    fn decoded_keyframe_without_clean_anchor_does_not_hold_family_gate_after_hold_window() {
        // 模拟：关键帧已解码，但没有 clean anchor 提交，且已超过短 hold 窗口。
        // 期望：新的 keyframe 请求不应被 same-family in-flight 长期压制。
        // 占坑判定先要求 anchor 仍在 RECOVERY_COMMAND_FAMILY_IN_FLIGHT_WINDOW_MS 内，
        // 否则不会进入 decoded hold 分支；时间轴需落在窗口内才能测「hold 过期解锁」。
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 3;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 800.0,
                sent_at_ms: Some(now_ms - 600.0),
                deadline_at_ms: Some(now_ms + 1_000.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(now_ms - 580.0),
                first_video_packet_rtp_timestamp: Some(123),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(now_ms - 580.0),
                // 解码点足够早，使 (now - decoded) > KEYFRAME_DECODED_PENDING_COMMIT_HOLD_MS
                first_keyframe_decoded_at_ms: Some(now_ms - 900.0),
                response_rtp_timestamp: Some(123),
                response_frame_seq: Some(456),
                response_verdict: Some("pending".to_string()),
            });
        // 未提交 clean anchor：video_anchor_clean_epoch=None
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestKeyframe".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 22,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let episode = snapshot
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.episode_id, 22);
        // transport 在无 peer/control 时仍会 transportDeferred；家族门控语义看 ledger（与 non_idr 用例一致）。
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("decodedPendingCommitExpired")
        );
        assert_ne!(ledger.coalescing_mode.as_deref(), Some("Merge"));
        assert!(ledger.command_detail.as_deref().map_or(true, |detail| {
            !detail.contains("sameFamilyCoalesced:keyframeInFlight")
        }));
    }

    #[test]
    fn non_idr_vcl_keyframe_response_does_not_hold_family_gate() {
        // 模拟：in-flight 期间 inspection 反映响应为 NonIdrVcl（bootstrap 不成立）。
        // 期望：新的 keyframe 请求不应被同 family in-flight 压制。
        // 这里不要求 transport 层一定能发出，只要求不会被 same-family gate 合并。
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 1;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 200.0,
                sent_at_ms: Some(now_ms - 150.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 1,
                frame_rtp_timestamp: Some(123),
                nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: true,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: Some(1920),
                sample_height: Some(1080),
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: true,
                observed_at_ms: now_ms - 10.0,
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
                    .to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestKeyframe".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 22,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let episode = snapshot
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.episode_id, 22);
        assert_eq!(episode.status, "deferred");
        assert_eq!(
            episode.response_verdict.as_deref(),
            Some("transportDeferred")
        );
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("bootstrapRejected:invalidBootstrap")
        );
        assert_ne!(ledger.coalescing_mode.as_deref(), Some("Merge"));
        assert!(ledger
            .command_detail
            .as_deref()
            .is_some_and(|detail| { !detail.contains("sameFamilyCoalesced:keyframeInFlight") }));
    }

    #[test]
    fn same_family_keyframe_coalescing_sets_ledger_fields() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 100.0,
                sent_at_ms: Some(now_ms - 80.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestKeyframe".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 22,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.command_result.as_deref(), Some("deferred"));
        assert_eq!(ledger.coalescing_mode.as_deref(), Some("Merge"));
        assert_eq!(
            ledger.recovery_primary_action.as_deref(),
            Some("requestKeyframe")
        );
    }

    #[test]
    fn recent_same_family_keyframe_episode_refreshes_when_not_in_flight() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "deferred".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 140.0,
                sent_at_ms: Some(now_ms - 120.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("transportDeferred".to_string()),
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestKeyframe".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 22,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.coalescing_mode.as_deref(), Some("Refresh"));
        assert!(ledger.command_detail.as_deref().is_some_and(|detail| {
            detail.contains("family=sameFamilyRefreshed:keyframeEpisode")
        }));
    }

    #[test]
    fn keyframe_inflight_upgrades_decoder_reset_and_sets_preempt_ledger_fields() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 1;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 100.0,
                sent_at_ms: Some(now_ms - 80.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 202,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestDecoderReset".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
            observation_id: 202,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.coalescing_mode.as_deref(), Some("Preempt"));
        assert_eq!(
            ledger.preempt_reason.as_deref(),
            Some("familyUpgrade:keyframeInFlight->decoderReset")
        );
        assert_eq!(
            ledger.recovery_primary_action.as_deref(),
            Some("requestDecoderReset")
        );
    }

    #[test]
    fn mismatched_family_does_not_inherit_transport_await_ledger_semantics() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 1;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 100.0,
                sent_at_ms: Some(now_ms - 80.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(123),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                observed_at_ms: now_ms - 10.0,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms - 10.0,
            },
            observed_at_ms: now_ms - 10.0,
        });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 303,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestDecoderReset".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
            observation_id: 303,
            reason: "displaySupplyCritical".to_string(),
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(
            ledger.recovery_primary_action.as_deref(),
            Some("requestDecoderReset")
        );
        assert_eq!(ledger.recovery_episode_stage, None);
        assert_eq!(ledger.gap_severity, None);
        assert_eq!(ledger.frame_value, None);
        assert_eq!(ledger.recovery_episode_progress_at_ms, None);
    }

    #[test]
    fn ledger_populates_episode_stage_gap_severity_and_frame_value() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 2;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 100.0,
                sent_at_ms: Some(now_ms - 80.0),
                deadline_at_ms: Some(now_ms + 500.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(123),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                observed_at_ms: now_ms - 10.0,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms - 10.0,
            },
            observed_at_ms: now_ms - 10.0,
        });

        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestKeyframe".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 22,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.recovery_episode_stage.as_deref(), Some("Sent"));
        assert_eq!(ledger.gap_severity.as_deref(), Some("AnchorGap"));
        assert_eq!(ledger.frame_value.as_deref(), Some("RecoveryAnchor"));
    }

    #[test]
    fn stalled_no_progress_unlocks_keyframe_and_marks_recovery_blocked_in_ledger() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 5;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 1_000.0,
                // 须在 in-flight 窗口内且超过 KEYFRAME_NO_PROGRESS_STALL_MS，才能命中 episodeStalledNoProgress
                sent_at_ms: Some(now_ms - 920.0),
                deadline_at_ms: Some(now_ms + 1_000.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(321),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                observed_at_ms: now_ms - 20.0,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms - 20.0,
            },
            observed_at_ms: now_ms - 20.0,
        });

        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestKeyframe".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 22,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let episode = snapshot
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.episode_id, 22);

        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("episodeStalledNoProgress")
        );
        assert_eq!(ledger.recovery_episode_stage.as_deref(), Some("Stalled"));
        assert_eq!(ledger.gap_severity.as_deref(), Some("RecoveryBlocked"));
        assert_ne!(ledger.coalescing_mode.as_deref(), Some("Merge"));
        assert!(ledger.command_detail.as_deref().map_or(true, |detail| {
            !detail.contains("sameFamilyCoalesced:keyframeInFlight")
        }));
    }

    // 主 gap 断言矩阵（对应 RFC：24573/30191/35010/41446）

    #[test]
    fn gap_24573_minor_gap_maps_to_continuity_value() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 24573,
            source_event: "nack-observation".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 24573,
                request_reason: None,
                request_kind: None,
                status: "requested".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: None,
                deadline_at_ms: None,
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 24573,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "none".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 24573,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.gap_severity.as_deref(), Some("MinorGap"));
        assert_eq!(ledger.frame_value.as_deref(), Some("Continuity"));
    }

    #[test]
    fn gap_30191_chain_broken_maps_to_recovery_anchor_value() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 30191,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                observed_at_ms: now_ms,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("referenceChainUnrecoverable".to_string()),
                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 30191,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 50.0,
                sent_at_ms: Some(now_ms - 40.0),
                deadline_at_ms: Some(now_ms + 800.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 30191,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "none".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 30191,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.gap_severity.as_deref(), Some("ChainBroken"));
        assert_eq!(ledger.frame_value.as_deref(), Some("RecoveryAnchor"));
    }

    #[test]
    fn gap_35010_maps_to_recovery_blocked_when_stalled_no_progress() {
        // 35010 关键样本：同 family 长时压制且无推进边沿，应进入 RecoveryBlocked 并允许解锁。
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 1;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 35010,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 1_000.0,
                sent_at_ms: Some(now_ms - 920.0),
                deadline_at_ms: Some(now_ms + 800.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 35010,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(2),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                observed_at_ms: now_ms,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 35010,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "none".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 35010,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.gap_severity.as_deref(), Some("RecoveryBlocked"));
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("episodeStalledNoProgress")
        );
        assert_eq!(ledger.recovery_episode_stage.as_deref(), Some("Stalled"));
    }

    #[test]
    fn gap_41446_post_recovery_reference_gap_maps_to_reference_value() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 2;
        stats.video_anchor_clean_epoch = Some(2);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 41446,
            source_event: "gap-repair-in-flight".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(3),
                frame_rtp_timestamp: None,
                frame_importance: Some("delta".to_string()),
                observed_at_ms: now_ms,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 1,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 500.0,
                sent_at_ms: Some(now_ms - 480.0),
                deadline_at_ms: Some(now_ms + 100.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(now_ms - 470.0),
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(now_ms - 470.0),
                first_keyframe_decoded_at_ms: Some(now_ms - 450.0),
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("on-time".to_string()),
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 41446,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "none".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: now_ms,
            });
        stats.recent_recovery_decision_ledgers =
            vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestKeyframe {
                observation_id: 41446,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.gap_severity.as_deref(), Some("ReferenceGap"));
        assert_eq!(ledger.frame_value.as_deref(), Some("Reference"));
    }

    #[test]
    fn command_result_updates_matching_recovery_decision_ledger() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        let matching_ledger = crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 88,
            state_before: "recovering".to_string(),
            state_after: "reconnecting".to_string(),
            input_signal: "liveness:livenessNoProgressTimeout".to_string(),
            gate_result: "pass".to_string(),
            action_selected: "requestReconnectCandidate".to_string(),
            frame_value: None,
            gap_severity: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: 10.0,
        };
        stats.latest_recovery_decision_ledger = Some(matching_ledger.clone());
        stats.recent_recovery_decision_ledgers.push(matching_ledger);
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.record_transport_command_status(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 88,
                reason: "recovering-stream".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
            CommandResultStatus::Deferred {
                reason: "pendingReason=existing".to_string(),
            },
        );

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("decision ledger");
        assert_eq!(ledger.command_result.as_deref(), Some("deferred"));
        assert_eq!(
            ledger.command_detail.as_deref(),
            Some("command=requestReconnectCandidate detail=pendingReason=existing")
        );
        let historical_ledger = snapshot
            .recent_recovery_decision_ledgers
            .iter()
            .find(|ledger| ledger.decision_id == 88)
            .expect("historical decision ledger");
        assert_eq!(
            historical_ledger.command_result.as_deref(),
            Some("deferred")
        );
    }

    #[test]
    fn command_result_updates_historical_ledger_when_latest_has_rotated() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recent_recovery_decision_ledgers.push(
            crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 88,
                state_before: "recovering".to_string(),
                state_after: "reconnecting".to_string(),
                input_signal: "liveness:livenessNoProgressTimeout".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestReconnectCandidate".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: 10.0,
            },
        );
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 99,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "none".to_string(),
                gate_result: "no-signal".to_string(),
                action_selected: "none".to_string(),
                frame_value: None,
                gap_severity: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: 11.0,
            });
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.record_transport_command_status(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 88,
                reason: "recovering-stream".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
            CommandResultStatus::Succeeded,
        );

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot
                .latest_recovery_decision_ledger
                .as_ref()
                .map(|ledger| ledger.decision_id),
            Some(99)
        );
        assert_eq!(
            snapshot
                .latest_recovery_decision_ledger
                .as_ref()
                .and_then(|ledger| ledger.command_result.as_deref()),
            None
        );
        let historical_ledger = snapshot
            .recent_recovery_decision_ledgers
            .iter()
            .find(|ledger| ledger.decision_id == 88)
            .expect("historical decision ledger");
        assert_eq!(
            historical_ledger.command_result.as_deref(),
            Some("succeeded")
        );
    }
}
