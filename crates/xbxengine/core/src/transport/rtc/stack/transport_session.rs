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
use crate::transport::rtc::recovery::timing::resolve_recovery_dynamic_timing;
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
const CAPABILITY_FEEDBACK_WARMING_REASON: &str = "capability:videoFeedbackWarming";
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
                    reason: CAPABILITY_FEEDBACK_WARMING_REASON.to_string(),
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
                            reason: CAPABILITY_FEEDBACK_WARMING_REASON.to_string(),
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
                episode.request_reason.as_deref() == Some("receiverWaitingKeyframe")
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
                    == Some("receiverLocalContinuation")
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
        let timing = resolve_recovery_dynamic_timing(stats, profile);
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
                ((now_ms - decoded_at_ms).max(0.0) >= timing.clean_anchor_commit_patience_window_ms)
                    .then_some("decodedPendingCommitExpired")
            })
    }
}

#[cfg(test)]
#[path = "transport_session.test.rs"]
mod tests;
