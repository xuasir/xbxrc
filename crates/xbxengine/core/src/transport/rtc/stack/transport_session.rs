use std::sync::{Arc, Mutex};

use crate::api::backend::{
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction, XbxEngineVideoBweObservation,
};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::{RtcConnectionService, VideoRecoveryRequestOutcome};
use crate::transport::rtc::executor::peer::{
    stage_reconnect_candidate, StageReconnectCandidateOutcome,
};
use crate::transport::rtc::facts::{
    CommandResultFact, CommandResultStatus, SessionCommand, TimerFact, TransportCommand,
    TransportFact,
};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::RtcMediaService;
use crate::XbxEngineRuntimeError;

const RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_CONTROL_PENDING: &str =
    "familyInFlight:controlChannelPending";
/// 视频 RTCP 反馈目标（TWCC/SSRC）尚未就绪，与「控制通道未就绪」同属可重试窗口，避免记 transportFailed。
const RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TRANSPORT_NOT_READY: &str =
    "familyDeferred:videoRtcpFeedbackTransportNotReady";
const RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TARGET_PENDING: &str =
    "familyDeferred:videoRtcpFeedbackTargetPending";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryCommandKind {
    RequestPli,
    RequestFir,
    RequestDecoderReset,
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
                        *observation_id, // 传入 observation_id 作为 decision_id
                        now_ms,
                    );
                let family_detail = match family_decision.as_ref() {
                    Some(reason) => {
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
                        return;
                    }
                    None => None,
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
                    family_detail,
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
                TransportCommand::RequestPli {
                    observation_id,
                    reason,
                } => {
                    let requested_at_ms = crate::transport::rtc::stats::now_ms_f64();
                    let (family_decision, family_semantics, family_semantic_detail) = self
                        .resolve_recovery_command_family_decision(
                            RecoveryCommandKind::RequestPli,
                            Some(reason.as_str()),
                            *observation_id, // 传入 observation_id 作为 decision_id
                            requested_at_ms,
                        );
                    if let Some(reason) = family_decision {
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
                    RuntimeStatsSink::new(self.runtime_stats.clone())
                        .record_picture_recovery_episode_requested(
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
                            connection.request_video_pli_with_outcome(self.runtime_stats)
                        });
                    let (command_status, semantic_detail) =
                        self.resolve_keyframe_command_status_from_result(&result);
                    if matches!(command_status, CommandResultStatus::Succeeded) {
                        if let Ok(outcome) = &result {
                            if let Some(action) = outcome.escalation_action_label() {
                                self.record_recovery_escalation_observation(
                                    *observation_id,
                                    reason.clone(),
                                    action,
                                    RecoveryAction::RequestPli,
                                );
                            }
                        }
                    }
                    match &command_status {
                        CommandResultStatus::Deferred { reason } => {
                            RuntimeStatsSink::new(self.runtime_stats.clone())
                                .record_picture_recovery_episode_deferred(requested_at_ms, reason);
                        }
                        CommandResultStatus::Failed { error } => {
                            RuntimeStatsSink::new(self.runtime_stats.clone())
                                .record_picture_recovery_episode_failed(requested_at_ms, error);
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
                TransportCommand::RequestFir {
                    observation_id,
                    reason,
                } => {
                    let requested_at_ms = crate::transport::rtc::stats::now_ms_f64();
                    let (family_decision, family_semantics, family_semantic_detail) = self
                        .resolve_recovery_command_family_decision(
                            RecoveryCommandKind::RequestFir,
                            Some(reason.as_str()),
                            *observation_id,
                            requested_at_ms,
                        );
                    if let Some(reason) = family_decision {
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
                    RuntimeStatsSink::new(self.runtime_stats.clone())
                        .record_picture_recovery_episode_requested(
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
                            connection.request_video_fir_with_outcome(self.runtime_stats)
                        });
                    let (command_status, semantic_detail) =
                        self.resolve_keyframe_command_status_from_result(&result);
                    if matches!(command_status, CommandResultStatus::Succeeded) {
                        if let Ok(outcome) = &result {
                            if let Some(action) = outcome.escalation_action_label() {
                                self.record_recovery_escalation_observation(
                                    *observation_id,
                                    reason.clone(),
                                    action,
                                    RecoveryAction::RequestFir,
                                );
                            }
                        }
                    }
                    match &command_status {
                        CommandResultStatus::Deferred { reason } => {
                            RuntimeStatsSink::new(self.runtime_stats.clone())
                                .record_picture_recovery_episode_deferred(requested_at_ms, reason);
                        }
                        CommandResultStatus::Failed { error } => {
                            RuntimeStatsSink::new(self.runtime_stats.clone())
                                .record_picture_recovery_episode_failed(requested_at_ms, error);
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

    fn update_recovery_decision_command_result(
        &self,
        command: &TransportCommand,
        status: &CommandResultStatus,
        observed_at_ms: f64,
        semantic_detail: Option<&str>,
    ) {
        let decision_id = match command {
            TransportCommand::RequestPli { observation_id, .. }
            | TransportCommand::RequestFir { observation_id, .. }
            | TransportCommand::RequestDecoderReset { observation_id, .. }
            | TransportCommand::RequestReconnectCandidate { observation_id, .. }
            | TransportCommand::SetTargetRembKbps { observation_id, .. } => *observation_id,
        };
        let command_name = match command {
            TransportCommand::RequestPli { .. } => "requestPli",
            TransportCommand::RequestFir { .. } => "requestFir",
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
            TransportCommand::RequestPli { .. } => "requestPli",
            TransportCommand::RequestFir { .. } => "requestFir",
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

    fn resolve_keyframe_command_status_from_result(
        &self,
        result: &Result<VideoRecoveryRequestOutcome, XbxEngineRuntimeError>,
    ) -> (CommandResultStatus, Option<String>) {
        match result {
            Ok(VideoRecoveryRequestOutcome::FeedbackTransportNotReady) => (
                CommandResultStatus::Deferred {
                    reason: RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TRANSPORT_NOT_READY
                        .to_string(),
                },
                None,
            ),
            Ok(VideoRecoveryRequestOutcome::FeedbackTargetPending) => (
                CommandResultStatus::Deferred {
                    reason: RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TARGET_PENDING.to_string(),
                },
                None,
            ),
            Ok(
                VideoRecoveryRequestOutcome::RequestedPli
                | VideoRecoveryRequestOutcome::RequestedFir,
            ) => (CommandResultStatus::Succeeded, None),
            Err(error) => {
                let error_text = error.to_string();
                if self.is_control_channel_not_ready_error(
                    RecoveryCommandKind::RequestPli,
                    &error_text,
                ) {
                    return (
                        CommandResultStatus::Deferred {
                            reason: RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_CONTROL_PENDING
                                .to_string(),
                        },
                        None,
                    );
                }
                if Self::is_keyframe_video_rtcp_feedback_target_pending_error(&error_text) {
                    return (
                        CommandResultStatus::Deferred {
                            reason: RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TARGET_PENDING
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
            RecoveryCommandKind::RequestPli => {
                error.contains("xbxEngineRtcControlChannelNotReadyForKeyframe")
            }
            RecoveryCommandKind::RequestFir => {
                error.contains("xbxEngineRtcControlChannelNotReadyForKeyframe")
            }
            RecoveryCommandKind::RequestDecoderReset => {
                error.contains("xbxEngineRtcControlChannelNotReadyForDecoderReset")
            }
        }
    }

    fn is_keyframe_video_rtcp_feedback_target_pending_error(error: &str) -> bool {
        error.contains("xbxEngineRtcVideoPliFeedbackTargetUnavailable")
            || error.contains("xbxEngineRtcVideoPliMediaSsrcUnavailable")
            || error.contains("xbxEngineRtcVideoFirFeedbackTargetUnavailable")
            || error.contains("xbxEngineRtcVideoFirMediaSsrcUnavailable")
    }

    fn resolve_recovery_command_family_decision(
        &self,
        _command_kind: RecoveryCommandKind,
        _reason_label: Option<&str>,
        decision_id: u64, // 添加 decision_id 参数
        now_ms: f64,
    ) -> (
        Option<String>,
        Option<RecoveryCommandDecisionSemantics>,
        Option<String>,
    ) {
        // 传输层降级为纯粹执行层：不做任何in-flight判定，完全信任决策层
        // 决策层（StateRecoveryCoordinator）已经处理了所有门控逻辑

        // 但是需要填充语义字段到 ledger，从 runtime_stats 中计算
        let semantics = RuntimeStatsSink::read_shared(&self.runtime_stats, |stats| {
            use crate::transport::rtc::session::facts::compute_recovery_facts;

            // 如果有 timeline observation，计算 facts 并提取语义字段
            let derived_unlock_reason = Self::derive_recovery_command_unlock_reason(stats, now_ms);
            if let Some(timeline) = stats.latest_video_timeline_observation.as_ref() {
                let facts = compute_recovery_facts(timeline, stats);

                // 使用 decision_id 查找对应的 ledger，而不是读取 latest
                let ledger = stats
                    .recent_recovery_decision_ledgers
                    .iter()
                    .find(|l| l.decision_id == decision_id);

                // 将 ledger 中的 String 转换为 &'static str
                let coalescing_mode = ledger.and_then(|l| {
                    l.coalescing_mode.as_deref().and_then(|s| match s {
                        "Merge" => Some("Merge"),
                        "Refresh" => Some("Refresh"),
                        "Preempt" => Some("Preempt"),
                        _ => None,
                    })
                });

                let recovery_primary_action = ledger.and_then(|l| {
                    l.recovery_primary_action.as_deref().and_then(|s| match s {
                        "requestPli" => Some("requestPli"),
                        "requestFir" => Some("requestFir"),
                        "requestDecoderReset" => Some("requestDecoderReset"),
                        "requestReconnect" => Some("requestReconnect"),
                        "suppress" => Some("suppress"),
                        _ => None,
                    })
                });
                let ledger_unlock_reason = ledger.and_then(|l| match l.unlock_reason.as_deref() {
                    Some("decodedPendingCommitExpired") => Some("decodedPendingCommitExpired"),
                    Some("bootstrapRejected:invalidBootstrap") => {
                        Some("bootstrapRejected:invalidBootstrap")
                    }
                    Some("continuationOnlyAwaitingIdr") => Some("continuationOnlyAwaitingIdr"),
                    Some("awaitingRecoveryAnchor") => Some("awaitingRecoveryAnchor"),
                    Some("episodeStalledNoProgress") => Some("episodeStalledNoProgress"),
                    _ => None,
                });

                Some(RecoveryCommandDecisionSemantics {
                    frame_value: facts.frame_value.map(|v| v.as_str()),
                    gap_severity: facts.gap_severity.map(|v| v.as_str()),
                    recovery_episode_stage: facts.recovery_progress_level.map(|v| v.as_str()),
                    recovery_episode_progress_at_ms: facts.recovery_episode_progress_at_ms,
                    coalescing_mode,
                    unlock_reason: ledger_unlock_reason.or(derived_unlock_reason),
                    preempt_reason: None, // preempt_reason 是动态字符串，暂时保持 None
                    recovery_primary_action,
                })
            } else if derived_unlock_reason.is_some() {
                Some(RecoveryCommandDecisionSemantics {
                    unlock_reason: derived_unlock_reason,
                    ..RecoveryCommandDecisionSemantics::default()
                })
            } else {
                None
            }
        })
        .flatten();

        (None, semantics, None)
    }

    fn derive_recovery_command_unlock_reason(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> Option<&'static str> {
        let episode = stats
            .recent_keyframe_request_episodes
            .iter()
            .chain(stats.latest_keyframe_request_episode.iter())
            .filter(|episode| {
                episode.request_reason.as_deref() == Some("transportAwaitRecoveryAnchor")
                    && episode.retired_at_ms.is_none()
            })
            .max_by(|left, right| {
                left.first_keyframe_decoded_at_ms
                    .is_some()
                    .cmp(&right.first_keyframe_decoded_at_ms.is_some())
                    .then_with(|| {
                        left.first_keyframe_packet_at_ms
                            .is_some()
                            .cmp(&right.first_keyframe_packet_at_ms.is_some())
                    })
                    .then_with(|| left.sent_at_ms.is_some().cmp(&right.sent_at_ms.is_some()))
                    .then_with(|| left.requested_at_ms.total_cmp(&right.requested_at_ms))
                    .then_with(|| left.episode_id.cmp(&right.episode_id))
            })?;
        let has_current_clean_anchor = stats.video_anchor_clean_epoch
            == Some(stats.transport_recovery_epoch)
            && stats.video_anchor_clean_observed_at_ms.is_some();
        if has_current_clean_anchor {
            return None;
        }
        if let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() {
            if inspection.observed_at_ms >= episode.requested_at_ms
                && !inspection.bootstrap_ready
                && inspection.admission_accepted
            {
                if matches!(
                    inspection.bootstrap_reject_reason.as_deref(),
                    Some(
                        "bootstrapMissingSps"
                            | "bootstrapMissingPps"
                            | "inspectionRejectInvalidSliceHeader"
                            | "NonIdrVcl"
                    )
                ) {
                    return Some("bootstrapRejected:invalidBootstrap");
                }
                if matches!(
                    inspection.bootstrap_reject_reason.as_deref(),
                    Some("bootstrapMissingIdr")
                ) && inspection.continuation_verdict.as_deref()
                    == Some("continuationAcceptedWhileAwaitingIdr")
                    && inspection.committed_sps_present
                    && inspection.committed_pps_present
                    && inspection.delta_continuation_ready
                {
                    return Some("continuationOnlyAwaitingIdr");
                }
            }
        }
        let profile =
            crate::transport::rtc::recovery::runtime_state::resolve_runtime_recovery_profile(stats);
        let anchor_candidate_stalled = matches!(
            episode.status.as_str(),
            "response-observed" | "packet-seen" | "decoded"
        ) && stats
            .latest_anchor_candidate_ledger
            .as_ref()
            .is_some_and(|candidate| {
                candidate.recovery_epoch == stats.transport_recovery_epoch
                    && candidate.observed_at_ms >= episode.requested_at_ms
                    && (now_ms - candidate.observed_at_ms).max(0.0)
                        <= profile.playback_recovered_track_progress_fresh_ms
                    && matches!(
                        candidate.source_event.as_str(),
                        "frame-await-recovery-anchor" | "gap-repair-in-flight" | "gap-resolved"
                    )
                    && matches!(
                        candidate.state,
                        crate::XbxEngineAnchorCandidateState::AwaitingRecovery
                            | crate::XbxEngineAnchorCandidateState::Repaired
                    )
            });
        if anchor_candidate_stalled {
            return Some("awaitingRecoveryAnchor");
        }
        episode
            .first_keyframe_decoded_at_ms
            .and_then(|decoded_at_ms| {
                ((now_ms - decoded_at_ms).max(0.0) >= profile.decoded_pending_commit_hold_ms)
                    .then_some("decodedPendingCommitExpired")
            })
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
        XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction, XbxEngineRuntimeError,
        XbxEngineVideoEscalationObservation,
    };

    use super::RtcTransportSessionBridge;
    use super::RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TARGET_PENDING;
    use super::RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TRANSPORT_NOT_READY;
    use crate::transport::rtc::connection::VideoRecoveryRequestOutcome;

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
            reason: "transportAwaitRecoveryAnchor".to_string(),
        });

        let msg = rx.recv().expect("local decoder reset message");
        match msg {
            DecodeMsg::LocalDecoderReset { reason, .. } => {
                assert_eq!(reason, "recoveryCommand:transportAwaitRecoveryAnchor");
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
            reason: "transportAwaitRecoveryAnchor".to_string(),
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
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
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

                ..Default::default()
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 42,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    .to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestDecoderReset".to_string(),
                frame_value: None,
                gap_severity: None,
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            reason: "transportAwaitRecoveryAnchor".to_string(),
        });

        let msg = rx.recv().expect("decoder reset should proceed");
        match msg {
            DecodeMsg::LocalDecoderReset { reason, .. } => {
                assert_eq!(reason, "recoveryCommand:transportAwaitRecoveryAnchor");
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
            reason: "transportAwaitRecoveryAnchor".to_string(),
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
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
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

                ..Default::default()
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 43,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    .to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestDecoderReset".to_string(),
                frame_value: None,
                gap_severity: None,
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            reason: "transportAwaitRecoveryAnchor".to_string(),
        });

        let msg = rx.recv().expect("decoder reset should proceed");
        match msg {
            DecodeMsg::LocalDecoderReset { reason, .. } => {
                assert_eq!(reason, "recoveryCommand:transportAwaitRecoveryAnchor");
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
            "transportAwaitRecoveryAnchor",
        ));
    }

    #[test]
    fn decoder_reset_is_deferred_when_control_reset_observation_is_recent() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 7;
        stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
            observation_id: 101,
            reason: "transportAwaitRecoveryAnchor".to_string(),
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
            reason: "transportAwaitRecoveryAnchor".to_string(),
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
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(SessionCommand::Transport(
            TransportCommand::RequestPli {
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
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        // 未提交 clean anchor：video_anchor_clean_epoch=None
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestPli".to_string(),
                frame_value: None,
                gap_severity: None,
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
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
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
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

                ..Default::default()
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    .to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestPli".to_string(),
                frame_value: None,
                gap_severity: None,
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
                observation_id: 22,
                reason: "transportAwaitRecoveryAnchor".to_string(),
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
    fn awaiting_recovery_anchor_after_packet_seen_does_not_hold_family_gate() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 4;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 41,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "packet-seen".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 520.0,
                sent_at_ms: Some(now_ms - 500.0),
                deadline_at_ms: Some(now_ms + 1_000.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(now_ms - 470.0),
                first_video_packet_rtp_timestamp: Some(0x1020_3300),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(now_ms - 470.0),
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(0x1020_3300),
                response_frame_seq: Some(77),
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: Some("packetSeen".to_string()),
                retired_at_ms: None,
            });
        stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
            recovery_epoch: 4,
            frame_rtp_timestamp: None,
            state: crate::XbxEngineAnchorCandidateState::AwaitingRecovery,
            source_event: "gap-repair-in-flight".to_string(),
            failure_reason: Some(crate::XbxEngineAnchorCandidateFailureReason::LocalRepairPending),
            observed_at_ms: now_ms - 40.0,
        });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 42,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    .to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestPli".to_string(),
                frame_value: None,
                gap_severity: None,
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
                observation_id: 42,
                reason: "transportAwaitRecoveryAnchor".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let episode = snapshot
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.episode_id, 42);
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("awaitingRecoveryAnchor")
        );
        assert_ne!(ledger.coalescing_mode.as_deref(), Some("Merge"));
        assert!(ledger.command_detail.as_deref().map_or(true, |detail| {
            !detail.contains("sameFamilyCoalesced:keyframeInFlight")
        }));
    }

    #[test]
    fn same_family_keyframe_coalescing_sets_ledger_fields() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 22,
                state_before: "detecting".to_string(),
                state_after: "detecting".to_string(),
                input_signal: "none".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestPli".to_string(),
                frame_value: None,
                gap_severity: None,
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: Some("Merge".to_string()),
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: Some("requestPli".to_string()),
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
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
            Some("requestPli")
        );
    }

    #[test]
    fn keyframe_inflight_upgrades_decoder_reset_and_sets_preempt_ledger_fields() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 1;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
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
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: Some("Preempt".to_string()),
                unlock_reason: None,
                preempt_reason: Some("familyUpgrade:keyframeInFlight->decoderReset".to_string()),
                recovery_primary_action: Some("requestDecoderReset".to_string()),
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            reason: "transportAwaitRecoveryAnchor".to_string(),
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
    fn ledger_populates_episode_stage_gap_severity_and_frame_value() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 2;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(123),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                budget_importance: None,

                evidence_importance: None,

                gap_dependency_confidence: None,

                observed_at_ms: now_ms - 10.0,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("awaitingRecoveryAnchor".to_string()),
                chain_break_evidence: None,

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
                action_selected: "requestPli".to_string(),
                frame_value: None,
                gap_severity: None,
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
                observation_id: 22,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(
            ledger.recovery_episode_stage.as_deref(),
            Some("WaitingResponse")
        );
        assert_eq!(ledger.gap_severity.as_deref(), Some("AnchorGap"));
        assert_eq!(ledger.frame_value.as_deref(), Some("RecoveryAnchor"));
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
                chain_break_evidence: None,

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
                lifecycle_phase: None,
                retired_at_ms: None,
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
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
                observation_id: 24573,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.gap_severity.as_deref(), Some("LowValueGap"));
        assert_eq!(ledger.frame_value.as_deref(), Some("Continuity"));
    }

    #[test]
    fn gap_30191_chain_broken_maps_to_recovery_anchor_value() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 30191,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                budget_importance: None,

                evidence_importance: None,

                gap_dependency_confidence: None,

                observed_at_ms: now_ms,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("referenceChainUnrecoverable".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 30191,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
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
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
                observation_id: 30191,
                reason: "transportAwaitRecoveryAnchor".to_string(),
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
    #[ignore = "等待 P1 完成：coordinator 行为需要调整"]
    fn gap_35010_maps_to_recovery_blocked_when_stalled_no_progress() {
        // 35010 关键样本：同 family 长时压制且无推进边沿，应进入 RecoveryBlocked 并允许解锁。
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 1;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 35010,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 35010,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                state: "open".to_string(),
                sequence: Some(2),
                frame_rtp_timestamp: None,
                frame_importance: Some("keyframe".to_string()),
                budget_importance: None,

                evidence_importance: None,

                gap_dependency_confidence: None,

                observed_at_ms: now_ms,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("awaitingRecoveryAnchor".to_string()),
                chain_break_evidence: None,

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
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
                observation_id: 35010,
                reason: "transportAwaitRecoveryAnchor".to_string(),
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
                budget_importance: None,

                evidence_importance: None,

                gap_dependency_confidence: None,

                observed_at_ms: now_ms,
            }),
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 1,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
                lifecycle_phase: None,
                retired_at_ms: None,
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
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
            TransportCommand::RequestPli {
                observation_id: 41446,
                reason: "ingressWaitKeyframe".to_string(),
            },
        ));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("ledger");
        assert_eq!(ledger.gap_severity.as_deref(), Some("RepairableGap"));
        assert_eq!(ledger.frame_value.as_deref(), Some("Continuity"));
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
            repairability: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
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
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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
                repairability: None,
                recovery_episode_stage: None,
                recovery_episode_progress_at_ms: None,
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
                recovery_primary_action: None,
                owner_surface_state: None,
                anchor_evidence: None,
                keyframe_episode_health: None,
                escalation_basis: None,
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

    #[test]
    fn keyframe_pli_feedback_target_unavailable_is_deferred_not_failed() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action);

        let (status, detail) = bridge.resolve_keyframe_command_status_from_result(&Err(
            XbxEngineRuntimeError::new("xbxEngineRtcVideoPliFeedbackTargetUnavailable"),
        ));

        assert_eq!(
            status,
            CommandResultStatus::Deferred {
                reason: RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TARGET_PENDING.to_string(),
            }
        );
        assert_eq!(detail, None);
    }

    #[test]
    fn keyframe_pli_transport_not_ready_is_deferred_not_failed() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action);

        let (status, detail) = bridge.resolve_keyframe_command_status_from_result(&Ok(
            VideoRecoveryRequestOutcome::FeedbackTransportNotReady,
        ));

        assert_eq!(
            status,
            CommandResultStatus::Deferred {
                reason: RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TRANSPORT_NOT_READY.to_string(),
            }
        );
        assert_eq!(detail, None);
    }

    #[test]
    fn trace_contract_feedback_target_pending_updates_ledger_with_family_deferred_reason() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        let decision = crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 42,
            state_before: "recovering".to_string(),
            state_after: "active-recovery".to_string(),
            input_signal: "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor".to_string(),
            gate_result: "pass:localProbe".to_string(),
            action_selected: "requestPli".to_string(),
            frame_value: Some("RecoveryAnchor".to_string()),
            gap_severity: Some("AnchorGap".to_string()),
            repairability: None,
            recovery_episode_stage: Some("WaitingResponse".to_string()),
            recovery_episode_progress_at_ms: Some(120.0),
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: Some("requestPli".to_string()),
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: 10_140.0,
        };
        stats.latest_recovery_decision_ledger = Some(decision.clone());
        stats.recent_recovery_decision_ledgers.push(decision);

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.record_transport_command_status(
            TransportCommand::RequestPli {
                observation_id: 42,
                reason: "transportAwaitRecoveryAnchor".to_string(),
            },
            CommandResultStatus::Deferred {
                reason: RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TARGET_PENDING.to_string(),
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
            Some("command=requestPli detail=familyDeferred:videoRtcpFeedbackTargetPending")
        );
    }

    #[test]
    fn trace_contract_feedback_transport_not_ready_updates_ledger_with_transport_reason() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        let decision = crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 43,
            state_before: "recovering".to_string(),
            state_after: "active-recovery".to_string(),
            input_signal: "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor".to_string(),
            gate_result: "pass:localProbe".to_string(),
            action_selected: "requestPli".to_string(),
            frame_value: Some("RecoveryAnchor".to_string()),
            gap_severity: Some("AnchorGap".to_string()),
            repairability: None,
            recovery_episode_stage: Some("WaitingResponse".to_string()),
            recovery_episode_progress_at_ms: Some(120.0),
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: Some("requestPli".to_string()),
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: 10_140.0,
        };
        stats.latest_recovery_decision_ledger = Some(decision.clone());
        stats.recent_recovery_decision_ledgers.push(decision);

        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.record_transport_command_status(
            TransportCommand::RequestPli {
                observation_id: 43,
                reason: "transportAwaitRecoveryAnchor".to_string(),
            },
            CommandResultStatus::Deferred {
                reason: RECOVERY_COMMAND_REASON_VIDEO_RTCP_FEEDBACK_TRANSPORT_NOT_READY.to_string(),
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
            Some("command=requestPli detail=familyDeferred:videoRtcpFeedbackTransportNotReady")
        );
    }
}
